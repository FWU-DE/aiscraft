//! Server-sent events, line by line, with a cap on event size.

use aiscraft_core::{Error, Result};

pub(crate) struct SseParser {
    line: Vec<u8>,
    data: String,
    has_data: bool,
    cap: usize,
}

impl SseParser {
    pub(crate) fn new(cap: usize) -> Self {
        Self { line: Vec::new(), data: String::new(), has_data: false, cap }
    }

    /// Feed bytes; returns the `data` of every event completed by them.
    pub(crate) fn feed(&mut self, mut bytes: &[u8]) -> Result<Vec<String>> {
        let mut out = Vec::new();
        while !bytes.is_empty() {
            let (part, rest, complete) = match bytes.iter().position(|&b| b == b'\n') {
                Some(i) => (bytes.get(..i).unwrap_or_default(), bytes.get(i + 1..).unwrap_or_default(), true),
                None => (bytes, &[][..], false),
            };
            if self.line.len() + part.len() > self.cap {
                return Err(too_big());
            }
            self.line.extend_from_slice(part);
            bytes = rest;
            if complete {
                self.end_line(&mut out)?;
            }
        }
        Ok(out)
    }

    /// The stream ended; an event without a trailing blank line still counts.
    pub(crate) fn finish(&mut self) -> Option<String> {
        let mut out = Vec::new();
        if !self.line.is_empty() {
            let _ = self.end_line(&mut out);
        }
        self.dispatch(&mut out);
        out.pop()
    }

    fn end_line(&mut self, out: &mut Vec<String>) -> Result<()> {
        let mut line = std::mem::take(&mut self.line);
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.is_empty() {
            self.dispatch(out);
            return Ok(());
        }
        let text = String::from_utf8_lossy(&line);
        if text.starts_with(':') {
            return Ok(());
        }
        let (field, value) = text.split_once(':').map(|(f, v)| (f, v.strip_prefix(' ').unwrap_or(v))).unwrap_or((&text, ""));
        if field == "data" {
            if self.has_data {
                self.data.push('\n');
            }
            self.data.push_str(value);
            self.has_data = true;
            if self.data.len() > self.cap {
                return Err(too_big());
            }
        }
        Ok(())
    }

    fn dispatch(&mut self, out: &mut Vec<String>) {
        if self.has_data {
            out.push(std::mem::take(&mut self.data));
            self.has_data = false;
        }
    }
}

fn too_big() -> Error {
    Error::protocol("Der Konnektor hat eine zu große Nachricht geschickt (mehr als 16 MiB).")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_split_across_chunks() {
        let mut p = SseParser::new(1024);
        assert!(p.feed(b"event: message\r\nid: 1\r\nda").unwrap().is_empty());
        assert_eq!(p.feed(b"ta: {\"a\":1}\r\n\r\n: comment\n\ndata: x\ndata:y\n\n").unwrap(), vec!["{\"a\":1}".to_string(), "x\ny".to_string()]);
        assert!(p.feed(b"id: only-an-id\n\n").unwrap().is_empty(), "priming events carry no data");
        assert!(p.feed(b"data: tail").unwrap().is_empty());
        assert_eq!(p.finish().as_deref(), Some("tail"));
    }

    #[test]
    fn oversized_events_are_refused() {
        let mut p = SseParser::new(8);
        assert!(p.feed(b"data: 0123456789").is_err());
        let mut p = SseParser::new(8);
        assert!(p.feed(b"data: 0123\ndata: 4567\n").is_err());
    }
}
