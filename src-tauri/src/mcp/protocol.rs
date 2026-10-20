use super::models::SUPPORTED_MCP_PROTOCOL_VERSIONS;

/// Represents an individual Server-Sent Event (SSE) parsed from a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event_type: Option<String>,
    pub data: String,
    pub id: Option<String>,
}

/// Parses complete SSE frames from an input buffer, handling split frames across chunks.
/// Returns a vector of parsed events and the byte count consumed from the buffer.
/// Unconsumed trailing bytes (incomplete event) are left for subsequent feeds.
pub fn parse_sse_frames(buffer: &str) -> (Vec<SseEvent>, usize) {
    let mut events = Vec::new();
    let mut consumed = 0;
    let bytes = buffer.as_bytes();

    let mut search_start = 0;
    while search_start < bytes.len() {
        // Look for event boundaries: "\r\n\r\n" or "\n\n"
        let boundary = find_sse_boundary(&bytes[search_start..]);
        let (rel_pos, delim_len) = match boundary {
            Some(b) => b,
            None => break, // No more complete events in buffer
        };

        let event_end = search_start + rel_pos;
        let event_str = match std::str::from_utf8(&bytes[consumed..event_end]) {
            Ok(s) => s,
            Err(_) => {
                // Skip invalid boundary search
                search_start += rel_pos + delim_len;
                continue;
            }
        };

        if let Some(event) = parse_single_sse_event(event_str) {
            events.push(event);
        }

        consumed = event_end + delim_len;
        search_start = consumed;
    }

    (events, consumed)
}

fn find_sse_boundary(slice: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < slice.len() {
        if slice[i] == b'\n' {
            if i + 1 < slice.len() && slice[i + 1] == b'\n' {
                return Some((i, 2));
            }
        } else if slice[i] == b'\r'
            && i + 3 < slice.len()
            && slice[i + 1] == b'\n'
            && slice[i + 2] == b'\r'
            && slice[i + 3] == b'\n'
        {
            return Some((i, 4));
        }
        i += 1;
    }
    None
}

fn parse_single_sse_event(event_block: &str) -> Option<SseEvent> {
    let mut event_type = None;
    let mut id = None;
    let mut data_lines = Vec::new();

    for line in event_block.lines() {
        let trimmed_line = line.trim_end_matches('\r');
        if trimmed_line.is_empty() || trimmed_line.starts_with(':') {
            // Empty line or SSE comment, skip
            continue;
        }

        let (field, value) = match trimmed_line.find(':') {
            Some(colon_idx) => {
                let f = &trimmed_line[..colon_idx];
                let rest = &trimmed_line[colon_idx + 1..];
                // Strip one optional leading space after colon per SSE spec
                let val = if rest.starts_with(' ') {
                    &rest[1..]
                } else {
                    rest
                };
                (f, val)
            }
            None => (trimmed_line, ""),
        };

        match field {
            "data" => data_lines.push(value),
            "event" => event_type = Some(value.to_string()),
            "id" => id = Some(value.to_string()),
            _ => {} // Ignore unknown fields
        }
    }

    if data_lines.is_empty() && event_type.is_none() && id.is_none() {
        None
    } else {
        Some(SseEvent {
            event_type,
            data: data_lines.join("\n"),
            id,
        })
    }
}

/// Searches parsed SSE events for a valid JSON-RPC 2.0 response matching an expected ID.
pub fn extract_jsonrpc_from_sse(
    events: &[SseEvent],
    expected_id: Option<u64>,
) -> Option<serde_json::Value> {
    for event in events {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&event.data) {
            if val.is_object() && val.get("jsonrpc").and_then(|v| v.as_str()) == Some("2.0") {
                if let Some(exp_id) = expected_id {
                    if val.get("id").and_then(|v| v.as_u64()) == Some(exp_id)
                        || val.get("error").is_some()
                    {
                        return Some(val);
                    }
                } else {
                    return Some(val);
                }
            }
        }
    }
    None
}

/// Checks whether an advertised MCP protocol version is in the supported spec set.
pub fn is_supported_mcp_version(version: &str) -> bool {
    SUPPORTED_MCP_PROTOCOL_VERSIONS.contains(&version)
}

/// Builds an MCP `initialize` JSON-RPC request.
pub fn build_initialize_request(id: u64, protocol_version: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": protocol_version,
            "capabilities": {},
            "clientInfo": {
                "name": "Yuki",
                "version": "0.1.0"
            }
        }
    })
}

/// Builds an MCP `notifications/initialized` JSON-RPC notification.
pub fn build_initialized_notification() -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    })
}

/// Builds an MCP `resources/read` JSON-RPC request.
pub fn build_resources_read_request(id: u64, uri: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "resources/read",
        "params": {
            "uri": uri
        }
    })
}

/// Builds an MCP `tools/call` request for `set_active_instance`.
pub fn build_set_active_instance_request(id: u64, instance_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "set_active_instance",
            "arguments": {
                "instance": instance_id
            }
        }
    })
}
