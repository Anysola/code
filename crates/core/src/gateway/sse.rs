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
}

impl Decoder {
    /// Feeds bytes; returns every complete block. Invalid UTF-8 is replaced.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Block> {
        self.buffer
            .push_str(&String::from_utf8_lossy(bytes).replace('\r', ""));
        let mut blocks = Vec::new();
        while let Some(end) = self.buffer.find("\n\n") {
            let raw: String = self.buffer.drain(..end + 2).collect();
            blocks.extend(parse_block(&raw[..end]));
        }
        blocks
    }

    /// The trailing partial block at end of stream.
    pub fn finish(&mut self) -> Vec<Block> {
        let rest = std::mem::take(&mut self.buffer);
        parse_block(rest.trim_end_matches('\n'))
            .into_iter()
            .collect()
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
