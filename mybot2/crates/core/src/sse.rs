//! A small Server-Sent Events reader over a byte stream.
//!
//! Chunks from the network split events, lines and even UTF-8 sequences at
//! arbitrary points, so bytes are buffered until a full event (blank line)
//! is in hand. `\r\n` and `\n` line endings are both accepted.

use futures_util::{Stream, StreamExt};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    /// Feed bytes; get back every event they completed.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let Some((end, sep)) = find_boundary(&self.buf) else { break };
            let raw: Vec<u8> = self.buf.drain(..end + sep).collect();
            let text = String::from_utf8_lossy(&raw[..end]);
            if let Some(ev) = parse_event(&text) {
                out.push(ev);
            }
        }
        out
    }

    /// Whatever is left when the stream ends without a trailing blank line.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if self.buf.is_empty() {
            return None;
        }
        let text = String::from_utf8_lossy(&self.buf).to_string();
        self.buf.clear();
        parse_event(&text)
    }
}

fn find_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < buf.len() {
        if buf[i..].starts_with(b"\r\n\r\n") {
            return Some((i, 4));
        }
        if buf[i..].starts_with(b"\n\n") {
            return Some((i, 2));
        }
        i += 1;
    }
    None
}

fn parse_event(text: &str) -> Option<SseEvent> {
    let mut ev = SseEvent::default();
    let mut data_lines = Vec::new();
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => ev.event = Some(value.to_string()),
            "data" => data_lines.push(value.to_string()),
            _ => {}
        }
    }
    if data_lines.is_empty() && ev.event.is_none() {
        return None;
    }
    ev.data = data_lines.join("\n");
    Some(ev)
}

/// Drive a response body through the parser, calling `on_event` per event.
/// Returns early (Ok) when `on_event` returns false.
pub async fn read_events<S, B, E, F>(mut body: S, mut on_event: F) -> Result<(), String>
where
    S: Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
    F: FnMut(SseEvent) -> bool,
{
    let mut parser = SseParser::default();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| format!("stream interrupted: {e}"))?;
        for ev in parser.push(chunk.as_ref()) {
            if !on_event(ev) {
                return Ok(());
            }
        }
    }
    if let Some(ev) = parser.finish() {
        on_event(ev);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_across_chunks_and_line_endings() {
        let mut p = SseParser::default();
        let whole = "event: a\ndata: {\"x\":1}\n\nevent: b\r\ndata: line1\r\ndata: line2\r\n\r\n: comment\n\ndata: tail";
        let mut got = Vec::new();
        for b in whole.as_bytes().chunks(3) {
            got.extend(p.push(b));
        }
        got.extend(p.finish());
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], SseEvent { event: Some("a".into()), data: "{\"x\":1}".into() });
        assert_eq!(got[1].data, "line1\nline2");
        assert_eq!(got[2].data, "tail");
    }

    #[test]
    fn multibyte_split() {
        let mut p = SseParser::default();
        let bytes = "data: héllo ✓\n\n".as_bytes();
        let mut got = Vec::new();
        for b in bytes.chunks(1) {
            got.extend(p.push(b));
        }
        assert_eq!(got[0].data, "héllo ✓");
    }
}
