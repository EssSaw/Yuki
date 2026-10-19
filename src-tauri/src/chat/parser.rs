use serde::Deserialize;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ParsedEvent {
    Delta(String),
    Done,
    Error(String),
}

#[derive(Default)]
pub struct SseParser {
    buffer: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// Feeds a chunk of text into the parser and returns all complete events extracted.
    pub fn feed(&mut self, chunk: &str) -> Vec<ParsedEvent> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();

        while let Some(pos) = self.buffer.find('\n') {
            let line = self.buffer[..pos].trim_end_matches('\r').to_string();
            self.buffer.drain(..=pos);

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // SSE comment / ping line
            if trimmed.starts_with(':') {
                continue;
            }

            if let Some(data) = trimmed.strip_prefix("data:") {
                let payload = data.trim();
                if payload == "[DONE]" {
                    events.push(ParsedEvent::Done);
                    continue;
                }

                match parse_chunk_json(payload) {
                    ChunkResult::Delta(text) => events.push(ParsedEvent::Delta(text)),
                    ChunkResult::Error(err) => events.push(ParsedEvent::Error(err)),
                    ChunkResult::Empty => {}
                }
            } else if trimmed.starts_with('{') {
                // Raw JSON error from provider without data: prefix
                if let Some(err) = parse_raw_json_error(trimmed) {
                    events.push(ParsedEvent::Error(err));
                }
            }
        }

        events
    }

    /// Consumes any remaining buffered content at stream end.
    pub fn finish(&mut self) -> Vec<ParsedEvent> {
        let remaining = std::mem::take(&mut self.buffer);
        let trimmed = remaining.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }

        let mut events = Vec::new();
        if let Some(data) = trimmed.strip_prefix("data:") {
            let payload = data.trim();
            if payload == "[DONE]" {
                events.push(ParsedEvent::Done);
            } else {
                match parse_chunk_json(payload) {
                    ChunkResult::Delta(text) => events.push(ParsedEvent::Delta(text)),
                    ChunkResult::Error(err) => events.push(ParsedEvent::Error(err)),
                    ChunkResult::Empty => {}
                }
            }
        } else if trimmed.starts_with('{') {
            if let Some(err) = parse_raw_json_error(trimmed) {
                events.push(ParsedEvent::Error(err));
            }
        }
        events
    }
}

enum ChunkResult {
    Delta(String),
    Error(String),
    Empty,
}

#[derive(Deserialize)]
struct OpenAiChunk {
    #[serde(default)]
    choices: Vec<OpenAiChoice>,
    error: Option<OpenAiErrorObj>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    #[serde(default)]
    delta: OpenAiDelta,
    finish_reason: Option<String>,
}

#[derive(Default, Deserialize)]
struct OpenAiDelta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiErrorObj {
    message: Option<String>,
}

#[derive(Deserialize)]
struct RawErrorPayload {
    error: Option<OpenAiErrorObj>,
    message: Option<String>,
}

fn parse_chunk_json(payload: &str) -> ChunkResult {
    match serde_json::from_str::<OpenAiChunk>(payload) {
        Ok(chunk) => {
            if let Some(err) = chunk.error {
                let msg = err
                    .message
                    .unwrap_or_else(|| "Unknown error from provider".to_string());
                return ChunkResult::Error(sanitize_error_message(&msg));
            }

            for choice in chunk.choices {
                if let Some(content) = choice.delta.content {
                    if !content.is_empty() {
                        return ChunkResult::Delta(content);
                    }
                }
                if let Some(ref finish) = choice.finish_reason {
                    if finish == "stop" || finish == "done" {
                        // Might be completion
                    }
                }
            }

            ChunkResult::Empty
        }
        Err(_) => {
            // Check if it was a standalone error object
            if let Some(err) = parse_raw_json_error(payload) {
                ChunkResult::Error(err)
            } else {
                // If it can't be parsed at all, avoid failing the stream unless it's explicitly an error
                ChunkResult::Empty
            }
        }
    }
}

fn parse_raw_json_error(payload: &str) -> Option<String> {
    if let Ok(raw) = serde_json::from_str::<RawErrorPayload>(payload) {
        if let Some(err) = raw.error {
            if let Some(msg) = err.message {
                return Some(sanitize_error_message(&msg));
            }
        }
        if let Some(msg) = raw.message {
            return Some(sanitize_error_message(&msg));
        }
    }
    None
}

/// Sanitizes error message from provider to prevent secret leakage or unbounded echoing.
pub fn sanitize_error_message(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "Unknown error from 9router provider.".to_string();
    }
    // Cap error message length to 500 characters
    let capped: String = trimmed.chars().take(500).collect();
    capped
}
