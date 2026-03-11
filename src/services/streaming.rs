use std::collections::HashMap;

/// Maximum buffer size before clearing (1MB)
const MAX_BUFFER_SIZE: usize = 1_048_576;

/// Simple SSE event parser that accumulates lines until a blank line, then yields the combined `data:` payload.
/// This follows the SSE spec: multiple `data:` lines per event are joined by `\n`.
/// Uses Vec<u8> buffer to handle split UTF-8 characters safely.
pub struct SseEventParser {
    buf: Vec<u8>,
    // Accumulates data: lines for the current event until blank line.
    cur_data_lines: Vec<String>,
}

impl SseEventParser {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(16 * 1024),
            cur_data_lines: Vec::with_capacity(4),
        }
    }

    /// Feed bytes and extract zero or more complete SSE event payloads (already joined).
    pub fn push_and_drain_events(&mut self, chunk: &[u8]) -> Vec<String> {
        // Check buffer size limit to prevent unbounded growth
        if self.buf.len() + chunk.len() > MAX_BUFFER_SIZE {
            log::warn!(
                "⚠️  SSE buffer exceeded {}MB limit (current: {} bytes, incoming: {} bytes). Clearing buffer to prevent memory exhaustion.",
                MAX_BUFFER_SIZE / 1_048_576,
                self.buf.len(),
                chunk.len()
            );
            // Clear buffer and start fresh with new chunk
            self.buf.clear();
            self.cur_data_lines.clear();
        }

        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();

        loop {
            // Find next newline
            let Some(pos) = self.buf.iter().position(|&b| b == b'\n') else {
                break;
            };

            // Take the line including the newline
            let line_bytes: Vec<u8> = self.buf.drain(..=pos).collect();

            // Trim newline and possible carriage return
            let mut len = line_bytes.len();
            if len > 0 && line_bytes[len - 1] == b'\n' {
                len -= 1;
                if len > 0 && line_bytes[len - 1] == b'\r' {
                    len -= 1;
                }
            }

            let trimmed = &line_bytes[..len];

            // Blank line => event terminator
            if trimmed.is_empty() {
                if !self.cur_data_lines.is_empty() {
                    let payload = self.cur_data_lines.join("\n");
                    self.cur_data_lines.clear();
                    out.push(payload);
                }
                continue;
            }

            // Only collect `data:` lines, ignore others (e.g., `event:`/`id:`)
            // Check for "data:" prefix (bytes: [100, 97, 116, 97, 58])
            if trimmed.starts_with(b"data:") {
                let data_content = &trimmed[5..];
                // Convert safely to string now that we have full lines
                // We use from_utf8_lossy here which is safe because we are at line boundaries
                // (assuming SSE lines are valid UTF-8, which they should be)
                let s = String::from_utf8_lossy(data_content)
                    .trim_start()
                    .to_string();
                self.cur_data_lines.push(s);
            }
        }

        out
    }

    /// Flush at end-of-stream (if the server doesn't send a final blank line).
    pub fn flush(mut self) -> Option<String> {
        // If there is data in buf that doesn't end in newline, we should try to process it
        if !self.buf.is_empty() {
            // Process remaining buffer as one last line
            let line_bytes = std::mem::take(&mut self.buf);
            let mut len = line_bytes.len();
            // Trim logic (though unlikely to have trailing \n here due to loop condition)
            if len > 0 && line_bytes[len - 1] == b'\n' {
                len -= 1;
                if len > 0 && line_bytes[len - 1] == b'\r' {
                    len -= 1;
                }
            }
            let trimmed = &line_bytes[..len];

            if trimmed.starts_with(b"data:") {
                let data_content = &trimmed[5..];
                let s = String::from_utf8_lossy(data_content)
                    .trim_start()
                    .to_string();
                self.cur_data_lines.push(s);
            }
        }

        if !self.cur_data_lines.is_empty() {
            let payload = self.cur_data_lines.join("\n");
            Some(payload)
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub struct ToolBuf {
    pub block_index: i32,
    pub id: Option<String>,
    pub name: Option<String>,
    pub pending_args: String,
    pub has_sent_start: bool,
}

pub type ToolsMap = HashMap<usize, ToolBuf>;

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // SseEventParser tests
    // ============================================================================

    #[test]
    fn test_sse_parser_single_event() {
        let mut parser = SseEventParser::new();
        let events = parser.push_and_drain_events(b"data: hello\n\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "hello");
    }

    #[test]
    fn test_sse_parser_multiple_events() {
        let mut parser = SseEventParser::new();
        let events = parser.push_and_drain_events(b"data: first\n\ndata: second\n\n");

        assert_eq!(events.len(), 2);
        assert_eq!(events[0], "first");
        assert_eq!(events[1], "second");
    }

    #[test]
    fn test_sse_parser_multiline_data() {
        let mut parser = SseEventParser::new();
        let events = parser.push_and_drain_events(b"data: line1\ndata: line2\n\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "line1\nline2");
    }

    #[test]
    fn test_sse_parser_incomplete_event() {
        let mut parser = SseEventParser::new();

        // First chunk - incomplete
        let events1 = parser.push_and_drain_events(b"data: incomplete");
        assert_eq!(events1.len(), 0);

        // Second chunk - completion
        let events2 = parser.push_and_drain_events(b"\n\n");
        assert_eq!(events2.len(), 1);
        assert_eq!(events2[0], "incomplete");
    }

    #[test]
    fn test_sse_parser_split_across_chunks() {
        let mut parser = SseEventParser::new();

        let events1 = parser.push_and_drain_events(b"data: ");
        assert_eq!(events1.len(), 0);

        let events2 = parser.push_and_drain_events(b"hello");
        assert_eq!(events2.len(), 0);

        let events3 = parser.push_and_drain_events(b"\n\n");
        assert_eq!(events3.len(), 1);
        assert_eq!(events3[0], "hello");
    }

    #[test]
    fn test_sse_parser_ignores_non_data_lines() {
        let mut parser = SseEventParser::new();
        let events = parser.push_and_drain_events(b"event: message\nid: 123\ndata: payload\n\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "payload");
    }

    #[test]
    fn test_sse_parser_empty_data() {
        let mut parser = SseEventParser::new();
        let events = parser.push_and_drain_events(b"data: \n\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "");
    }

    #[test]
    fn test_sse_parser_multiple_blank_lines() {
        let mut parser = SseEventParser::new();
        let events = parser.push_and_drain_events(b"data: test\n\n\n\ndata: next\n\n");

        assert_eq!(events.len(), 2);
        assert_eq!(events[0], "test");
        assert_eq!(events[1], "next");
    }

    #[test]
    fn test_sse_parser_carriage_return() {
        let mut parser = SseEventParser::new();
        let events = parser.push_and_drain_events(b"data: test\r\n\r\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "test");
    }

    #[test]
    fn test_sse_parser_done_message() {
        let mut parser = SseEventParser::new();
        let events = parser.push_and_drain_events(b"data: [DONE]\n\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "[DONE]");
    }

    #[test]
    fn test_sse_parser_json_payload() {
        let mut parser = SseEventParser::new();
        let events = parser.push_and_drain_events(b"data: {\"key\":\"value\"}\n\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0], r#"{"key":"value"}"#);
    }

    #[test]
    fn test_sse_parser_whitespace_in_data() {
        let mut parser = SseEventParser::new();
        let events = parser.push_and_drain_events(b"data:   spaced content  \n\n");

        assert_eq!(events.len(), 1);
        // Leading space after colon is stripped
        assert_eq!(events[0], "spaced content  ");
    }

    #[test]
    fn test_sse_parser_empty_input() {
        let mut parser = SseEventParser::new();
        let events = parser.push_and_drain_events(b"");

        assert_eq!(events.len(), 0);
    }

    #[test]
    fn test_sse_parser_flush_with_incomplete_line() {
        let mut parser = SseEventParser::new();
        // Push data with newline but no blank terminator
        let _ = parser.push_and_drain_events(b"data: incomplete\n");

        // flush() consumes the parser and returns accumulated data lines
        let flushed = parser.flush();
        // The "data: incomplete\n" was parsed, data line was accumulated
        assert_eq!(flushed, Some("incomplete".to_string()));
    }

    #[test]
    fn test_sse_parser_flush_with_partial_line() {
        let mut parser = SseEventParser::new();
        // Push incomplete data (no newline at all)
        let _ = parser.push_and_drain_events(b"data: partial");

        // flush() handles the remaining bytes
        let flushed = parser.flush();
        assert_eq!(flushed, Some("partial".to_string()));
    }

    #[test]
    fn test_sse_parser_flush_empty() {
        let parser = SseEventParser::new();

        let flushed = parser.flush();
        assert_eq!(flushed, None);
    }

    #[test]
    fn test_sse_parser_flush_after_complete_event() {
        let mut parser = SseEventParser::new();
        let _events = parser.push_and_drain_events(b"data: complete\n\n");

        let flushed = parser.flush();
        assert_eq!(flushed, None);
    }

    #[test]
    fn test_sse_parser_large_chunk() {
        let mut parser = SseEventParser::new();

        // Create a large but valid event
        let large_data = "x".repeat(1000);
        let input = format!("data: {}\n\n", large_data);

        let events = parser.push_and_drain_events(input.as_bytes());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].len(), 1000);
    }

    #[test]
    fn test_sse_parser_buffer_limit_exceeded() {
        let mut parser = SseEventParser::new();

        // Create a chunk that exceeds MAX_BUFFER_SIZE (1MB)
        let huge_data = vec![b'x'; MAX_BUFFER_SIZE + 1000];

        // This should trigger the buffer clear warning
        let events = parser.push_and_drain_events(&huge_data);

        // Buffer should be cleared and start fresh
        // Since there's no newline, no events returned
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn test_sse_parser_real_world_openai_chunk() {
        let mut parser = SseEventParser::new();
        let chunk = b"data: {\"id\":\"chatcmpl-123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"index\":0}]}\n\n";

        let events = parser.push_and_drain_events(chunk);
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("chatcmpl-123"));
        assert!(events[0].contains("Hello"));
    }

    #[test]
    fn test_sse_parser_real_world_anthropic_chunk() {
        let mut parser = SseEventParser::new();
        let chunk = b"data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n";

        let events = parser.push_and_drain_events(chunk);
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("content_block_delta"));
        assert!(events[0].contains("Hello"));
    }

    #[test]
    fn test_sse_parser_utf8_content() {
        let mut parser = SseEventParser::new();
        let events = parser.push_and_drain_events("data: Hello 世界 🌍\n\n".as_bytes());

        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "Hello 世界 🌍");
    }

    #[test]
    fn test_sse_parser_sequential_events_no_gap() {
        let mut parser = SseEventParser::new();

        // Three events with no gap between terminating blank line and next event
        let input = b"data: first\n\ndata: second\n\ndata: third\n\n";
        let events = parser.push_and_drain_events(input);

        assert_eq!(events.len(), 3);
        assert_eq!(events[0], "first");
        assert_eq!(events[1], "second");
        assert_eq!(events[2], "third");
    }

    #[test]
    fn test_sse_parser_split_utf8_character() {
        let mut parser = SseEventParser::new();

        // Euro symbol '€' is 3 bytes: [226, 130, 172]
        let euro = "€";

        // Send partial bytes
        let part1 = format!("data: price: {}", euro);
        let bytes1 = part1.as_bytes();
        // Split right in the middle of the Euro symbol
        let (chunk1, chunk2) = bytes1.split_at(bytes1.len() - 1);

        let events1 = parser.push_and_drain_events(chunk1);
        assert_eq!(events1.len(), 0);

        let chunk2_with_newline = [chunk2, b"\n\n"].concat();
        let events2 = parser.push_and_drain_events(&chunk2_with_newline);

        assert_eq!(events2.len(), 1);
        assert_eq!(events2[0], "price: €");
    }
}
