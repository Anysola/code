//! Server-sent events from `/v1/code/messages`: Anthropic's stream plus the gateway's
//! `anysola_usage` block. Blocks are split on blank lines after dropping `\r`; comment lines
//! (`: keepalive`) carry nothing.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Event(SseEvent),
    /// The gateway's closing `anysola_usage` event, if it parsed.
    Usage(Option<super::limits::UsageEvent>),
}

#[derive(Default)]
pub struct Decoder {
    buffer: String,
    /// Bytes of a UTF-8 sequence cut by the last chunk boundary, decoded with the next chunk.
    partial: Vec<u8>,
}

impl Decoder {
    /// Feeds bytes; returns every complete block. A multi-byte character split across chunks is
    /// joined before decoding; invalid UTF-8 is replaced.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Block> {
        self.partial.extend_from_slice(bytes);
        let complete = utf8_boundary(&self.partial);
        let text = String::from_utf8_lossy(&self.partial[..complete]).replace('\r', "");
        self.partial.drain(..complete);
        self.buffer.push_str(&text);
        let mut blocks = Vec::new();
        while let Some(end) = self.buffer.find("\n\n") {
            let raw: String = self.buffer.drain(..end + 2).collect();
            blocks.extend(parse_block(&raw[..end]));
        }
        blocks
    }

    /// The trailing partial block at end of stream.
    pub fn finish(&mut self) -> Vec<Block> {
        if !self.partial.is_empty() {
            let rest = std::mem::take(&mut self.partial);
            self.buffer.push_str(&String::from_utf8_lossy(&rest));
        }
        let rest = std::mem::take(&mut self.buffer);
        parse_block(rest.trim_end_matches('\n'))
            .into_iter()
            .collect()
    }
}

/// Length of the longest prefix that does not end inside a UTF-8 sequence: an incomplete
/// trailing sequence (at most 3 bytes) waits for the next chunk.
fn utf8_boundary(bytes: &[u8]) -> usize {
    match std::str::from_utf8(bytes) {
        Ok(_) => bytes.len(),
        Err(error) => match error.error_len() {
            // Invalid bytes are decoded (and replaced) now; only a cut-off sequence waits.
            Some(_) => bytes.len(),
            None => error.valid_up_to(),
        },
    }
}

fn parse_block(text: &str) -> Option<Block> {
    if text.starts_with("event: anysola_usage") {
        let usage = text
            .lines()
            .find_map(|line| line.strip_prefix("data:"))
            .and_then(|data| serde_json::from_str::<serde_json::Value>(data.trim()).ok())
            .and_then(|value| super::limits::UsageEvent::from_wire(&value));
        return Some(Block::Usage(usage));
    }
    let mut event = None;
    let mut data: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => event = Some(value.to_string()),
            "data" => data.push(value),
            _ => {}
        }
    }
    if event.is_none() && data.is_empty() {
        return None;
    }
    Some(Block::Event(SseEvent {
        event,
        data: data.join("\n"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multibyte_characters_survive_chunk_boundaries() {
        let text = "şeker çğİ 🎉";
        let event = format!("event: content_block_delta\ndata: {{\"text\":\"{text}\"}}\n\n");
        let bytes = event.as_bytes();
        // Every split point, including inside each multi-byte sequence.
        for cut in 0..bytes.len() {
            let mut decoder = Decoder::default();
            let mut blocks = decoder.push(&bytes[..cut]);
            blocks.extend(decoder.push(&bytes[cut..]));
            blocks.extend(decoder.finish());
            assert_eq!(
                blocks,
                vec![Block::Event(SseEvent {
                    event: Some("content_block_delta".into()),
                    data: format!("{{\"text\":\"{text}\"}}"),
                })],
                "cut at byte {cut}"
            );
        }
        // One byte at a time.
        let mut decoder = Decoder::default();
        let mut blocks = Vec::new();
        for byte in bytes {
            blocks.extend(decoder.push(std::slice::from_ref(byte)));
        }
        assert_eq!(blocks.len(), 1);
        // The stream ends inside a sequence.
        let mut decoder = Decoder::default();
        let mut cut = b"data: ".to_vec();
        cut.push("ş".as_bytes()[0]);
        assert!(decoder.push(&cut).is_empty());
        assert_eq!(
            decoder.finish(),
            vec![Block::Event(SseEvent {
                event: None,
                data: "\u{FFFD}".into()
            })],
            "a cut-off sequence at end of stream is replaced, not lost"
        );
    }

    #[test]
    fn splits_events_and_strips_usage() {
        let mut decoder = Decoder::default();
        let mut blocks = decoder.push(
            b": keepalive\n\nevent: message_start\r\ndata: {\"a\":1}\r\n\r\nevent: anysola_us",
        );
        blocks.extend(decoder.push(b"age\ndata: {\"type\":\"anysola_usage\",\"fiveHourPercentUsed\":0.5,\"fiveHour\":{\"percent\":42,\"resetsAt\":null},\"weekly\":{\"percent\":18,\"resetsAt\":null}}\n\nevent: anysola_usage\ndata: {}\n\nevent: message_stop\ndata: {}"));
        blocks.extend(decoder.finish());
        assert_eq!(
            blocks,
            vec![
                Block::Event(SseEvent {
                    event: Some("message_start".into()),
                    data: "{\"a\":1}".into()
                }),
                Block::Usage(Some(super::super::limits::UsageEvent {
                    five_hour_percent_used: 0.5,
                    five_hour: super::super::limits::Window {
                        percent: 42,
                        resets_at: None
                    },
                    weekly: super::super::limits::Window {
                        percent: 18,
                        resets_at: None
                    },
                })),
                Block::Usage(None),
                Block::Event(SseEvent {
                    event: Some("message_stop".into()),
                    data: "{}".into()
                }),
            ]
        );
    }
}
