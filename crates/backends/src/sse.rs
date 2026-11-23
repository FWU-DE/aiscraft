//! Server-sent events, as OpenAI and Anthropic stream them.

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

const MAX_EVENT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some((end, sep)) = find_blank_line(&self.buf) {
            let block: Vec<u8> = self.buf.drain(..end + sep).collect();
            let text = String::from_utf8_lossy(block.get(..end).unwrap_or_default()).into_owned();
            let mut ev = SseEvent::default();
            let mut data_lines = Vec::new();
            for line in text.lines() {
                if line.starts_with(':') {
                    continue;
                }
                let (field, value) = line.split_once(':').map(|(f, v)| (f, v.strip_prefix(' ').unwrap_or(v))).unwrap_or((line, ""));
                match field {
                    "event" => ev.event = Some(value.to_string()),
                    "data" => data_lines.push(value.to_string()),
                    _ => {}
                }
            }
            if !data_lines.is_empty() || ev.event.is_some() {
                ev.data = data_lines.join("\n");
                out.push(ev);
            }
        }
        if self.buf.len() > MAX_EVENT_BYTES {
            return Err("Server-Ereignis zu groß".into());
        }
        Ok(out)
    }
}

fn find_blank_line(b: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < b.len() {
        if b.get(i..i + 2) == Some(b"\n\n") {
            return Some((i, 2));
        }
        if b.get(i..i + 4) == Some(b"\r\n\r\n") {
            return Some((i, 4));
        }
        i += 1;
    }
    None
}

/// Turn an SSE response into a stream of events. `handle` maps one SSE event to zero or more
/// stream events; after it yields [`StreamEvent::Done`] the stream ends.
pub fn event_stream<F>(resp: reqwest::Response, handle: F) -> crate::EventStream
where
    F: FnMut(SseEvent) -> aiscraft_core::Result<Vec<aiscraft_core::StreamEvent>> + Send + 'static,
{
    use aiscraft_core::{Error, StreamEvent};
    use futures::StreamExt;

    struct St<F> {
        body: futures::stream::BoxStream<'static, reqwest::Result<bytes::Bytes>>,
        parser: SseParser,
        queue: std::collections::VecDeque<aiscraft_core::Result<StreamEvent>>,
        handle: F,
        finished: bool,
    }
    let st = St { body: resp.bytes_stream().boxed(), parser: SseParser::default(), queue: Default::default(), handle, finished: false };
    futures::stream::unfold(st, |mut st| async move {
        loop {
            if let Some(item) = st.queue.pop_front() {
                if matches!(item, Ok(StreamEvent::Done) | Err(_)) {
                    st.finished = true;
                    st.queue.clear();
                }
                return Some((item, st));
            }
            if st.finished {
                return None;
            }
            match st.body.next().await {
                Some(Ok(bytes)) => match st.parser.feed(&bytes) {
                    Ok(events) => {
                        for ev in events {
                            match (st.handle)(ev) {
                                Ok(out) => st.queue.extend(out.into_iter().map(Ok)),
                                Err(e) => st.queue.push_back(Err(e)),
                            }
                        }
                    }
                    Err(e) => st.queue.push_back(Err(Error::protocol(e))),
                },
                Some(Err(e)) => st.queue.push_back(Err(crate::http::net_err(e))),
                // Body ended without an explicit end marker: treat as done.
                None => st.queue.push_back(Ok(StreamEvent::Done)),
            }
        }
    })
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_split_events() {
        let input = b"event: message_start\ndata: {\"a\":1}\n\n: ping\n\ndata: x\r\ndata: y\r\n\r\ndata: [DONE]\n\n";
        for split in [1, 3, 100] {
            let mut p = SseParser::default();
            let mut evs = Vec::new();
            for c in input.chunks(split) {
                evs.extend(p.feed(c).unwrap());
            }
            assert_eq!(evs.len(), 3, "split {split}");
            assert_eq!(evs[0].event.as_deref(), Some("message_start"));
            assert_eq!(evs[1].data, "x\ny");
            assert_eq!(evs[2].data, "[DONE]");
        }
    }
}
