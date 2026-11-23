//! React Flight (the RSC wire format), the subset a server action response uses.
//!
//! A response is a sequence of rows `<hex id>:<payload>`. Most payloads are one line of JSON
//! ended by `\n`. A payload may instead start with a tag letter (`I`, `H`, `E`, `R`, `C`, …),
//! and some tags (`T` text, typed arrays) carry a hex byte length and a comma, then exactly that
//! many bytes with no newline. The parser is incremental: feed it bytes as they arrive.

use std::collections::HashMap;

use serde_json::Value;

/// One complete row.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// A JSON model value.
    Json { id: u64, value: Value },
    /// A `T` text row (a long string, or a string chunk of a stream).
    Text { id: u64, text: String },
    /// Any other tagged row: `tag` is the letter, `payload` the rest of the line.
    Tagged { id: u64, tag: char, payload: String },
    /// A length-prefixed binary row other than `T`.
    Binary { id: u64, tag: char, bytes: Vec<u8> },
}

impl Row {
    pub fn id(&self) -> u64 {
        match self {
            Row::Json { id, .. } | Row::Text { id, .. } | Row::Tagged { id, .. } | Row::Binary { id, .. } => *id,
        }
    }
}

/// Tags followed by `<hex length>,<bytes>`, as in React's `processBinaryChunk`.
const LENGTH_TAGS: &[u8] = b"TAOobUSsLlGgMmV";
/// Upper bound for one length-prefixed row, so a hostile length cannot make us buffer forever.
const MAX_ROW_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct RowParser {
    buf: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError(pub String);

impl RowParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append bytes and return every row that is now complete.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Row>, ParseError> {
        self.buf.extend_from_slice(bytes);
        let mut rows = Vec::new();
        while let Some((row, used)) = self.next_row()? {
            self.buf.drain(..used);
            if let Some(row) = row {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    /// Bytes buffered but not yet a full row (non-empty at end of input means truncation).
    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    /// Try to cut one row off the front of the buffer. `Ok(None)` = need more bytes.
    /// The inner `Option<Row>` is `None` for blank lines.
    fn next_row(&self) -> Result<Option<(Option<Row>, usize)>, ParseError> {
        let b = &self.buf;
        if b.is_empty() {
            return Ok(None);
        }
        if b.first() == Some(&b'\n') {
            return Ok(Some((None, 1)));
        }
        let Some(colon) = b.iter().position(|&c| c == b':') else {
            if b.len() > 32 || b.contains(&b'\n') {
                return Err(ParseError(format!("row without id: {:?}", String::from_utf8_lossy(b.get(..b.len().min(80)).unwrap_or_default()))));
            }
            return Ok(None);
        };
        let id_str = std::str::from_utf8(b.get(..colon).unwrap_or_default()).map_err(|_| ParseError("row id is not UTF-8".into()))?;
        let id = u64::from_str_radix(id_str.trim(), 16).map_err(|_| ParseError(format!("bad row id {id_str:?}")))?;
        let rest_start = colon + 1;
        let Some(&tag) = b.get(rest_start) else { return Ok(None) };

        if LENGTH_TAGS.contains(&tag) {
            let len_start = rest_start + 1;
            let Some(comma_rel) = b.get(len_start..).unwrap_or_default().iter().position(|&c| c == b',') else {
                return Ok(None);
            };
            let len_str = std::str::from_utf8(b.get(len_start..len_start + comma_rel).unwrap_or_default()).map_err(|_| ParseError("bad length".into()))?;
            let len = usize::from_str_radix(len_str, 16).map_err(|_| ParseError(format!("bad length {len_str:?}")))?;
            if len > MAX_ROW_BYTES {
                return Err(ParseError(format!("row of {len} bytes is too large")));
            }
            let data_start = len_start + comma_rel + 1;
            let data_end = data_start.checked_add(len).ok_or_else(|| ParseError("length overflow".into()))?;
            let Some(data) = b.get(data_start..data_end) else { return Ok(None) };
            let row = if tag == b'T' {
                Row::Text { id, text: String::from_utf8_lossy(data).into_owned() }
            } else {
                Row::Binary { id, tag: tag as char, bytes: data.to_vec() }
            };
            return Ok(Some((Some(row), data_end)));
        }

        let Some(nl_rel) = b.get(rest_start..).unwrap_or_default().iter().position(|&c| c == b'\n') else {
            return Ok(None);
        };
        let line = String::from_utf8_lossy(b.get(rest_start..rest_start + nl_rel).unwrap_or_default()).into_owned();
        let used = rest_start + nl_rel + 1;
        let is_tag = tag.is_ascii_uppercase() || tag == b'#' || tag == b'r' || tag == b'x';
        let row = if is_tag {
            Row::Tagged { id, tag: tag as char, payload: line.get(1..).unwrap_or_default().to_string() }
        } else {
            let value = serde_json::from_str(&line).map_err(|e| ParseError(format!("row {id:x}: {e}")))?;
            Row::Json { id, value }
        };
        Ok(Some((Some(row), used)))
    }
}

/// What a server action response means to its caller.
#[derive(Debug, Clone, PartialEq)]
pub enum ActionEvent {
    /// The action's return value, with `$<id>` references to other JSON rows resolved
    /// (one level of nesting is enough for AIS.chat's `{ stream, messageId, error }`).
    Result(Value),
    /// A string chunk of the returned stream.
    Chunk(String),
    /// The stream closed normally.
    Closed,
    /// The action or its stream failed on the server.
    Error(String),
}

/// Decodes a server action response whose result object holds a `ReadableStream` in the field
/// named by `stream_field`. Rows may arrive in any order; chunks for the stream are held back
/// until the result has said which row id the stream is.
#[derive(Debug)]
pub struct ActionDecoder {
    parser: RowParser,
    stream_field: String,
    json: HashMap<u64, Value>,
    result_id: Option<u64>,
    result_sent: bool,
    stream_id: Option<u64>,
    held: HashMap<u64, Vec<ActionEvent>>,
    closed: bool,
}

impl ActionDecoder {
    pub fn new(stream_field: &str) -> Self {
        Self {
            parser: RowParser::new(),
            stream_field: stream_field.to_string(),
            json: HashMap::new(),
            result_id: None,
            result_sent: false,
            stream_id: None,
            held: HashMap::new(),
            closed: false,
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<ActionEvent>, ParseError> {
        let rows = self.parser.feed(bytes)?;
        let mut out = Vec::new();
        for row in rows {
            self.take_row(row, &mut out);
        }
        Ok(out)
    }

    /// Call at end of input: reports truncation or a stream that never closed.
    pub fn finish(&mut self) -> Vec<ActionEvent> {
        if self.closed {
            return Vec::new();
        }
        if !self.result_sent {
            return vec![ActionEvent::Error("Antwort des Servers ohne Ergebnis".into())];
        }
        if self.stream_id.is_some() {
            return vec![ActionEvent::Error("Die Antwort brach vorzeitig ab".into())];
        }
        Vec::new()
    }

    fn take_row(&mut self, row: Row, out: &mut Vec<ActionEvent>) {
        let id = row.id();
        match row {
            Row::Json { value, .. } => {
                if id == 0 && self.result_id.is_none() {
                    // Next.js action envelope: {"a": "$@1", "f": …, "b": buildId}
                    if let Some(r) = value.get("a").and_then(Value::as_str).and_then(parse_ref) {
                        self.result_id = Some(r);
                    } else {
                        self.result_id = Some(0);
                        self.json.insert(0, value);
                    }
                } else if Some(id) == self.stream_id {
                    out.push(ActionEvent::Chunk(stream_value_text(&value)));
                    return;
                } else if self.stream_id.is_none() && self.json.contains_key(&id) {
                    // A second JSON row for an id we already have: a stream chunk whose stream
                    // we have not identified yet.
                    self.held.entry(id).or_default().push(ActionEvent::Chunk(stream_value_text(&value)));
                    return;
                } else {
                    self.json.insert(id, value);
                }
                self.try_emit_result(out);
            }
            Row::Text { text, .. } => self.stream_item(id, ActionEvent::Chunk(text), out),
            Row::Tagged { tag, payload, .. } => match tag {
                'R' | 'r' | 'X' | 'x' => {
                    // Stream start: nothing to emit, the result references it.
                }
                'C' => self.stream_item(id, ActionEvent::Closed, out),
                'E' => {
                    let msg = serde_json::from_str::<Value>(&payload)
                        .ok()
                        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_string))
                        .filter(|m| !m.is_empty())
                        .unwrap_or_else(|| "Fehler auf dem Server".to_string());
                    if Some(id) == self.result_id || Some(id) == self.stream_id || self.stream_id.is_none() {
                        out.push(ActionEvent::Error(msg));
                        self.closed = true;
                    }
                }
                _ => {} // I, H, D, W, … are hints and debug info.
            },
            Row::Binary { .. } => {}
        }
    }

    fn stream_item(&mut self, id: u64, ev: ActionEvent, out: &mut Vec<ActionEvent>) {
        if Some(id) == self.stream_id {
            if ev == ActionEvent::Closed {
                self.closed = true;
            }
            out.push(ev);
        } else if self.stream_id.is_none() {
            self.held.entry(id).or_default().push(ev);
        }
    }

    fn try_emit_result(&mut self, out: &mut Vec<ActionEvent>) {
        if self.result_sent {
            return;
        }
        let Some(rid) = self.result_id else { return };
        let Some(raw) = self.json.get(&rid).cloned() else { return };
        let resolved = self.resolve(&raw, 0);
        if let Some(sid) = raw.get(&self.stream_field).and_then(Value::as_str).and_then(parse_ref) {
            self.stream_id = Some(sid);
        }
        self.result_sent = true;
        out.push(ActionEvent::Result(resolved));
        if let Some(sid) = self.stream_id {
            for ev in self.held.remove(&sid).unwrap_or_default() {
                if ev == ActionEvent::Closed {
                    self.closed = true;
                }
                out.push(ev);
            }
        } else {
            self.closed = true; // No stream: the result is all there is.
        }
    }

    fn resolve(&self, v: &Value, depth: usize) -> Value {
        if depth > 8 {
            return v.clone();
        }
        match v {
            Value::String(s) => {
                if let Some(id) = parse_ref(s) {
                    if let Some(target) = self.json.get(&id) {
                        return self.resolve(target, depth + 1);
                    }
                    return v.clone();
                }
                Value::String(unescape(s))
            }
            Value::Array(a) => Value::Array(a.iter().map(|x| self.resolve(x, depth + 1)).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), self.resolve(x, depth + 1))).collect()),
            other => other.clone(),
        }
    }
}

/// `"$1f"` or `"$@1f"` → `0x1f`. Other `$` forms (`$$`, `$undefined`, `$D…`) are not references.
fn parse_ref(s: &str) -> Option<u64> {
    let rest = s.strip_prefix('$')?;
    let rest = rest.strip_prefix('@').unwrap_or(rest);
    if rest.is_empty() || !rest.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(rest, 16).ok()
}

fn unescape(s: &str) -> String {
    s.strip_prefix("$$").map(|r| format!("${r}")).unwrap_or_else(|| s.to_string())
}

fn stream_value_text(v: &Value) -> String {
    match v {
        Value::String(s) => unescape(s),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn decode_all(input: &[u8], split: usize) -> Vec<ActionEvent> {
        let mut d = ActionDecoder::new("stream");
        let mut out = Vec::new();
        for chunk in input.chunks(split.max(1)) {
            out.extend(d.feed(chunk).unwrap());
        }
        out.extend(d.finish());
        out
    }

    const RESPONSE: &str = "0:{\"a\":\"$@1\",\"f\":\"\",\"b\":\"build\"}\n1:{\"stream\":\"$2\",\"messageId\":\"m1\"}\n2:R\n2:T5,Hallo2:\" Welt\"\n2:T8,\u{1e}{\"a\":1}2:C\n";

    #[test]
    fn decodes_streamed_action_at_any_split() {
        for split in [1, 2, 3, 7, 1000] {
            let ev = decode_all(RESPONSE.as_bytes(), split);
            assert_eq!(
                ev,
                vec![
                    ActionEvent::Result(json!({"stream": "$2", "messageId": "m1"})),
                    ActionEvent::Chunk("Hallo".into()),
                    ActionEvent::Chunk(" Welt".into()),
                    ActionEvent::Chunk("\u{1e}{\"a\":1}".into()),
                    ActionEvent::Closed,
                ],
                "split {split}"
            );
        }
    }

    #[test]
    fn utf8_text_rows_count_bytes() {
        // "Grüße" is 7 bytes in UTF-8.
        let r = "0:{\"a\":\"$@1\"}\n1:{\"stream\":\"$2\"}\n2:R\n2:T7,Grüße2:C\n";
        let ev = decode_all(r.as_bytes(), 1);
        assert!(ev.contains(&ActionEvent::Chunk("Grüße".into())));
        assert_eq!(ev.last(), Some(&ActionEvent::Closed));
    }

    #[test]
    fn chunks_before_result_are_held() {
        let r = "2:R\n2:T2,ab0:{\"a\":\"$@1\"}\n1:{\"stream\":\"$2\",\"error\":\"$3\"}\n3:{\"name\":\"X\"}\n2:C\n";
        let ev = decode_all(r.as_bytes(), 4);
        assert!(matches!(ev.first(), Some(ActionEvent::Result(_))));
        assert!(ev.contains(&ActionEvent::Chunk("ab".into())));
    }

    #[test]
    fn server_error_row() {
        let r = "0:{\"a\":\"$@1\"}\n1:E{\"digest\":\"123\",\"message\":\"TokenPointsExceeded\"}\n";
        let ev = decode_all(r.as_bytes(), 5);
        assert_eq!(ev, vec![ActionEvent::Error("TokenPointsExceeded".into())]);
    }

    #[test]
    fn truncated_stream_is_reported() {
        let r = "0:{\"a\":\"$@1\"}\n1:{\"stream\":\"$2\"}\n2:R\n2:T5,Hal";
        let ev = decode_all(r.as_bytes(), 3);
        assert!(matches!(ev.last(), Some(ActionEvent::Error(_))));
    }

    #[test]
    fn hostile_input_never_panics() {
        let cases: &[&[u8]] = &[b"zz:{}\n", b"0:Tffffffffffffffffff,x", b"0:T10,", b"\xff\xfe:\n", b"0:{\"a\n", b"0:\xff\n", b"1:Tz,abc", b":\n"];
        for c in cases {
            let mut d = ActionDecoder::new("stream");
            let _ = d.feed(c);
            let _ = d.finish();
        }
    }
}
