//! Server-sent events, as the WHATWG HTML standard defines the `text/event-stream` format:
//! lines end in CR, LF or CRLF; `data:` lines accumulate; a blank line dispatches the event;
//! lines starting with `:` are comments. Only `event` and `data` matter to the providers here.

/// One dispatched event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` field, if any.
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SseError {
    #[error("an event in the stream is larger than {0} bytes")]
    TooLarge(usize),
    #[error("the stream is not valid UTF-8")]
    Encoding,
}

/// Incremental decoder: feed it bytes as they arrive.
#[derive(Debug)]
pub struct SseDecoder {
    max_event_bytes: usize,
    /// Bytes of an unfinished line.
    line: Vec<u8>,
    /// The previous chunk ended in CR, so a leading LF belongs to that line ending.
    after_cr: bool,
    event: Option<String>,
    data: String,
    has_data: bool,
}

impl SseDecoder {
    #[must_use]
    pub fn new(max_event_bytes: usize) -> Self {
        Self {
            max_event_bytes,
            line: Vec::new(),
            after_cr: false,
            event: None,
            data: String::new(),
            has_data: false,
        }
    }

    /// Decodes `bytes`, returning the events they complete.
    ///
    /// # Errors
    ///
    /// When an event outgrows the limit or a line is not UTF-8.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, SseError> {
        let mut events = Vec::new();
        for &byte in bytes {
            if self.after_cr {
                self.after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            match byte {
                b'\n' | b'\r' => {
                    self.after_cr = byte == b'\r';
                    let line = std::mem::take(&mut self.line);
                    if let Some(event) = self.line_done(&line)? {
                        events.push(event);
                    }
                }
                _ => {
                    if self.line.len() + self.data.len() >= self.max_event_bytes {
                        return Err(SseError::TooLarge(self.max_event_bytes));
                    }
                    self.line.push(byte);
                }
            }
        }
        Ok(events)
    }

    fn line_done(&mut self, line: &[u8]) -> Result<Option<SseEvent>, SseError> {
        if line.is_empty() {
            return Ok(self.dispatch());
        }
        let line = std::str::from_utf8(line).map_err(|_| SseError::Encoding)?;
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            // `id` and `retry` concern reconnection, which these one-shot requests never do.
            _ => {}
        }
        Ok(None)
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        if !self.has_data {
            return None;
        }
        self.has_data = false;
        Some(SseEvent {
            event,
            data: std::mem::take(&mut self.data),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_in_pieces(input: &[u8], piece: usize) -> Vec<SseEvent> {
        let mut decoder = SseDecoder::new(1024);
        input
            .chunks(piece)
            .flat_map(|chunk| decoder.push(chunk).expect("decodes"))
            .collect()
    }

    #[test]
    fn events_survive_any_chunking() {
        let input = b": keep-alive\r\nevent: message_start\r\ndata: {\"a\":1}\r\n\r\n\
                      data: first\ndata:second\n\nevent: ping\n\ndata: x\r\rdata: last\n\n";
        let expected = vec![
            SseEvent {
                event: Some("message_start".into()),
                data: "{\"a\":1}".into(),
            },
            SseEvent {
                event: None,
                data: "first\nsecond".into(),
            },
            SseEvent {
                event: None,
                data: "x".into(),
            },
            SseEvent {
                event: None,
                data: "last".into(),
            },
        ];
        for piece in [1, 2, 3, 7, input.len()] {
            assert_eq!(decode_in_pieces(input, piece), expected, "piece {piece}");
        }
    }

    #[test]
    fn an_unfinished_event_is_not_dispatched() {
        let mut decoder = SseDecoder::new(1024);
        assert_eq!(decoder.push(b"data: partial\n").expect("decodes"), vec![]);
    }

    #[test]
    fn oversized_events_and_bad_encoding_are_errors() {
        let mut decoder = SseDecoder::new(16);
        assert_eq!(
            decoder.push(b"data: 0123456789\ndata: 0123456789\n"),
            Err(SseError::TooLarge(16))
        );
        let mut decoder = SseDecoder::new(16);
        assert_eq!(decoder.push(b"data: \xff\n"), Err(SseError::Encoding));
        // Multi-byte characters split across chunks are fine.
        let mut decoder = SseDecoder::new(64);
        let text = "data: é\n\n".as_bytes();
        let mut events = decoder.push(&text[..7]).expect("decodes");
        events.extend(decoder.push(&text[7..]).expect("decodes"));
        assert_eq!(events[0].data, "é");
    }
}
