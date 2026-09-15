//! Minimal server-sent events parser for provider streams.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed bytes; returns every complete event they finished.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();

        loop {
            let Some((end, skip)) = find_blank_line(&self.buffer) else { break };
            let block: Vec<u8> = self.buffer.drain(..end + skip).collect();
            let text = String::from_utf8_lossy(&block[..end]);
            if let Some(event) = parse_block(&text) {
                events.push(event);
            }
        }
        events
    }
}

fn find_blank_line(buffer: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < buffer.len() {
        if buffer[i..].starts_with(b"\r\n\r\n") {
            return Some((i, 4));
        }
        if buffer[i..].starts_with(b"\n\n") {
            return Some((i, 2));
        }
        i += 1;
    }
    None
}

fn parse_block(text: &str) -> Option<SseEvent> {
    let mut event = SseEvent::default();
    let mut data_lines: Vec<&str> = Vec::new();
    for raw in text.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => event.event = Some(value.to_string()),
            "data" => data_lines.push(value),
            _ => {}
        }
    }
    if data_lines.is_empty() && event.event.is_none() {
        return None;
    }
    event.data = data_lines.join("\n");
    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_events_across_chunks() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"data: {\"a\":1}\n").is_empty());
        let events = parser.push(b"\nevent: done\ndata: x\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "{\"a\":1}");
        assert_eq!(events[1].event.as_deref(), Some("done"));
        assert_eq!(events[1].data, "x");
    }
}
