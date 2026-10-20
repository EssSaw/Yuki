use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::client::{
    MAX_HISTORY_CHARS, MAX_INPUT_CHARS, build_http_client, fetch_models, prepare_bounded_messages,
    validate_endpoint,
};
use super::commands::{load_chat_config, save_chat_config_atomic};
use super::credentials::{MockSecretStore, SERVICE_NAME, SecretStore, USER_NAME};
use super::history::{
    delete_conversation_from_disk, get_conversations_dir, is_matching_conversation_file,
    is_symlink_or_reparse, list_conversations_from_disk, load_conversation_from_disk,
    save_conversation_atomic, validate_conversation_id,
};
use super::models::{
    ChatConfig, ChatMessage, ConversationDetail, MessageRole, MessageStatus, ToolActivityPayload,
    ToolActivityStatus,
};
use super::parser::{ParsedEvent, SseParser};
use super::state::ChatState;
use super::tools::{
    MAX_CUMULATIVE_TOOL_OUTPUT_BYTES, MAX_TOOL_ARGUMENTS_BYTES, MAX_TOOL_RESULT_BYTES,
    MAX_TOOL_ROUNDS, MAX_TOOLS_PER_ROUND, MAX_TOTAL_TOOL_CALLS, TOOL_DISCOVER_GAME_PROJECTS,
    ToolCallAccumulator, execute_tool, get_chat_tools, reauthorize_tool_result,
    safe_running_summary, summarize_tool_output,
};
use crate::workspace::settings::{add_project_root_path, toggle_project_provider_sharing_impl};

/// Test helper providing an isolated sandbox in target/test_temp.
/// Strictly avoids /tmp, /var/tmp, or user directories.
struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(name: &str) -> Self {
        let unique = format!(
            "chat_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let base = dunce::canonicalize(std::env::current_dir().unwrap())
            .unwrap()
            .join("target")
            .join("test_temp")
            .join(unique);
        std::fs::create_dir_all(&base).unwrap();
        Self { root: base }
    }

    fn config_dir(&self) -> PathBuf {
        let dir = self.root.join("config");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn data_root(&self) -> PathBuf {
        let dir = self.root.join("data_root");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn new_dir(&self, name: &str) -> PathBuf {
        let dir = self.root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// ---------------------------------------------------------------------------
// 1. Credentials Abstraction Tests
// ---------------------------------------------------------------------------

#[test]
fn test_mock_secret_store_isolation() {
    let store = MockSecretStore::new();

    // Initially empty
    assert_eq!(store.get_api_key(SERVICE_NAME, USER_NAME).unwrap(), None);

    // Set key
    store
        .set_api_key(SERVICE_NAME, USER_NAME, "test-secret-key-1234")
        .unwrap();
    assert_eq!(
        store.get_api_key(SERVICE_NAME, USER_NAME).unwrap(),
        Some("test-secret-key-1234".to_string())
    );

    // Delete key
    store.delete_api_key(SERVICE_NAME, USER_NAME).unwrap();
    assert_eq!(store.get_api_key(SERVICE_NAME, USER_NAME).unwrap(), None);
}

#[test]
fn test_chat_config_never_persists_api_key() {
    let env = TestEnv::new("config_no_key");
    let cfg_dir = env.config_dir();

    let config = ChatConfig {
        base_url: "http://127.0.0.1:20128/v1".to_string(),
        model: "gpt-4o-mini".to_string(),
    };
    save_chat_config_atomic(&cfg_dir, &config).unwrap();

    let reloaded = load_chat_config(&cfg_dir).unwrap();
    assert_eq!(reloaded, config);

    // Raw file content check
    let raw = std::fs::read_to_string(cfg_dir.join("chat_settings.json")).unwrap();
    assert!(!raw.contains("api_key"));
    assert!(!raw.contains("apiKey"));
}

// ---------------------------------------------------------------------------
// 2. Endpoint Validation & Redirect Policy Tests
// ---------------------------------------------------------------------------

#[test]
fn test_endpoint_validation_rules() {
    // Loopback HTTP allowed
    assert!(validate_endpoint("http://localhost:20128/v1").is_ok());
    assert!(validate_endpoint("http://127.0.0.1:8080/v1").is_ok());
    assert!(validate_endpoint("http://[::1]:11434").is_ok());

    // Non-loopback HTTP rejected
    let res = validate_endpoint("http://api.openai.com/v1");
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("loopback"));

    let res_ip = validate_endpoint("http://192.168.1.50:8000");
    assert!(res_ip.is_err());

    // Remote HTTPS allowed
    assert!(validate_endpoint("https://api.openai.com/v1").is_ok());
    assert!(validate_endpoint("https://router.internal.company.com/v1").is_ok());

    // Other schemes rejected
    assert!(validate_endpoint("ftp://localhost:21").is_err());
    assert!(validate_endpoint("file:///C:/secret").is_err());
    assert!(validate_endpoint("").is_err());
}

#[tokio::test]
async fn test_http_client_redirect_policy_none() {
    let client = build_http_client().unwrap();

    // Start a local server that responds with a redirect
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let resp = "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9999/leaked\r\nContent-Length: 0\r\n\r\n";
            let _ = socket.write_all(resp.as_bytes()).await;
        }
    });

    let res = client
        .get(format!("http://{addr}/test"))
        .send()
        .await
        .unwrap();

    // Since Policy::none() is used, the client should receive the 302 directly, NOT follow it!
    assert_eq!(res.status(), reqwest::StatusCode::FOUND);
}

// ---------------------------------------------------------------------------
// 3. SSE Parser Tests (Robust Chunk Boundaries, [DONE], Errors)
// ---------------------------------------------------------------------------

#[test]
fn test_sse_parser_finish_reason_stop_without_done() {
    let mut parser = SseParser::new();
    let chunk1 = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello world\"}}]}\n\n";
    let events1 = parser.feed(chunk1);
    assert_eq!(events1, vec![ParsedEvent::Delta("Hello world".to_string())]);

    // Terminal chunk with finish_reason: "stop" and NO [DONE]
    let chunk2 = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
    let events2 = parser.feed(chunk2);
    assert_eq!(events2, vec![ParsedEvent::Done]);
}

#[test]
fn test_sse_parser_finish_reason_with_delta_in_same_chunk() {
    let mut parser = SseParser::new();
    // Delta and finish_reason in the exact same chunk
    let chunk = "data: {\"choices\":[{\"delta\":{\"content\":\"Final delta.\"},\"finish_reason\":\"stop\"}]}\n\n";
    let events = parser.feed(chunk);
    assert_eq!(
        events,
        vec![
            ParsedEvent::Delta("Final delta.".to_string()),
            ParsedEvent::Done
        ]
    );
}

#[test]
fn test_sse_parser_finish_reason_in_tail_buffer_without_newline() {
    let mut parser = SseParser::new();
    // Incomplete line without trailing newline when stream ends
    let chunk =
        "data: {\"choices\":[{\"delta\":{\"content\":\"Tail text\"},\"finish_reason\":\"stop\"}]}";
    let fed = parser.feed(chunk);
    assert!(fed.is_empty()); // Buffered

    let finished = parser.finish();
    assert_eq!(
        finished,
        vec![
            ParsedEvent::Delta("Tail text".to_string()),
            ParsedEvent::Done
        ]
    );
}

#[test]
fn test_sse_parser_finish_reason_split_across_chunks() {
    let mut parser = SseParser::new();
    let p1 = "data: {\"choices\":[{\"delta\":{},\"finish_";
    let p2 = "reason\":\"stop\"}]}\n\n";

    assert!(parser.feed(p1).is_empty());
    assert_eq!(parser.feed(p2), vec![ParsedEvent::Done]);
}

#[test]
fn test_sse_parser_valid_and_invalid_finish_reasons() {
    let mut parser = SseParser::new();

    // Standard and compatible completion reasons should yield Done
    for reason in [
        "stop",
        "length",
        "tool_calls",
        "content_filter",
        "function_call",
        "done",
        "end_turn",
        "stop_sequence",
        "eos",
        "STOP",
    ] {
        let chunk = format!(
            "data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"{reason}\"}}]}}\n\n"
        );
        let events = parser.feed(&chunk);
        assert_eq!(
            events,
            vec![ParsedEvent::Done],
            "Failed for reason: {reason}"
        );
    }

    // Explicit error finish reason should yield Error event
    let err_chunk = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"error\"}]}\n\n";
    let err_events = parser.feed(err_chunk);
    assert!(matches!(err_events.first(), Some(ParsedEvent::Error(_))));

    // Null or empty finish_reason should NOT yield Done
    let null_chunk = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":null}]}\n\n";
    assert!(parser.feed(null_chunk).is_empty());
}

#[test]
fn test_sse_parser_standard_and_done() {
    let mut parser = SseParser::new();
    let chunk1 = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n";
    let events1 = parser.feed(chunk1);
    assert_eq!(events1, vec![ParsedEvent::Delta("Hello".to_string())]);

    let chunk2 = "data: {\"choices\":[{\"delta\":{\"content\":\" world!\"}}]}\n\ndata: [DONE]\n\n";
    let events2 = parser.feed(chunk2);
    assert_eq!(
        events2,
        vec![ParsedEvent::Delta(" world!".to_string()), ParsedEvent::Done]
    );
}

#[test]
fn test_sse_parser_split_across_chunk_boundaries() {
    let mut parser = SseParser::new();

    // Chunk split right inside the JSON content
    let p1 = "data: {\"choices\":[{\"delta\":{\"content\":\"Part";
    let p2 = "ial and split\"}}]}\n\n";

    let ev1 = parser.feed(p1);
    assert!(ev1.is_empty()); // Incomplete line, buffered

    let ev2 = parser.feed(p2);
    assert_eq!(
        ev2,
        vec![ParsedEvent::Delta("Partial and split".to_string())]
    );

    // Chunk split right inside [DONE]
    let d1 = "data: [DO";
    let d2 = "NE]\n";
    assert!(parser.feed(d1).is_empty());
    assert_eq!(parser.feed(d2), vec![ParsedEvent::Done]);
}

#[test]
fn test_sse_parser_ignores_heartbeats_and_empty_deltas() {
    let mut parser = SseParser::new();
    let raw =
        ": ping\n\ndata: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n: heartbeat\n\n";
    let events = parser.feed(raw);
    assert!(events.is_empty());
}

#[test]
fn test_sse_parser_handles_json_error_payloads() {
    let mut parser = SseParser::new();
    let err_chunk = "data: {\"error\":{\"message\":\"Rate limit exceeded\"}}\n\n";
    let events = parser.feed(err_chunk);
    assert_eq!(
        events,
        vec![ParsedEvent::Error("Rate limit exceeded".to_string())]
    );

    // Standalone JSON error without data: prefix
    let raw_err = "{\"error\":{\"message\":\"Model not found\"}}\n";
    let events2 = parser.feed(raw_err);
    assert_eq!(
        events2,
        vec![ParsedEvent::Error("Model not found".to_string())]
    );
}

// ---------------------------------------------------------------------------
// 4. History Storage, Path Traversal, and Corruption Resilience Tests
// ---------------------------------------------------------------------------

#[test]
fn test_path_traversal_rejected_for_conversation_ids() {
    assert!(validate_conversation_id("../secret").is_err());
    assert!(validate_conversation_id("..\\secret").is_err());
    assert!(validate_conversation_id("/etc/passwd").is_err());
    assert!(validate_conversation_id("C:\\Windows\\System32").is_err());
    assert!(validate_conversation_id("invalid-uuid").is_err());
    assert!(validate_conversation_id("").is_err());

    // Valid UUID v4 passes
    let valid = uuid::Uuid::new_v4().to_string();
    assert!(validate_conversation_id(&valid).is_ok());
}

#[test]
fn test_conversation_atomic_persistence_and_listing() {
    let env = TestEnv::new("history_atomic");
    let data_root = env.data_root();

    let id1 = uuid::Uuid::new_v4().to_string();
    let id2 = uuid::Uuid::new_v4().to_string();

    let conv1 = ConversationDetail {
        id: id1.clone(),
        title: "First Conversation".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            content: "Hello from 1".to_string(),
            status: MessageStatus::Complete,
        }],
        ..Default::default()
    };

    let conv2 = ConversationDetail {
        id: id2.clone(),
        title: "Second Conversation".to_string(),
        updated_at: "2026-10-19T11:00:00Z".to_string(), // newer
        messages: vec![ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            content: "Hello from 2".to_string(),
            status: MessageStatus::Complete,
        }],
        ..Default::default()
    };

    save_conversation_atomic(&data_root, &conv1).unwrap();
    save_conversation_atomic(&data_root, &conv2).unwrap();

    // Verify loaded conversation matches exactly
    let loaded1 = load_conversation_from_disk(&data_root, &id1).unwrap();
    assert_eq!(loaded1, conv1);

    // Verify listing sorts by updatedAt descending
    let list = list_conversations_from_disk(&data_root).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, id2); // Newer
    assert_eq!(list[1].id, id1); // Older
}

#[test]
fn test_history_corruption_and_future_schema_resilience() {
    let env = TestEnv::new("corruption_resilience");
    let data_root = env.data_root();
    let conv_dir = get_conversations_dir(&data_root).unwrap();

    let valid_id = uuid::Uuid::new_v4().to_string();
    let corrupt_id = uuid::Uuid::new_v4().to_string();

    // 1. Write a valid conversation with future schema properties (extra fields)
    let future_schema_json = format!(
        r#"{{
            "id": "{valid_id}",
            "title": "Future Conversation",
            "updatedAt": "2026-10-19T12:00:00Z",
            "futureField": "unexpected",
            "messages": [
                {{
                    "id": "{valid_id}",
                    "role": "user",
                    "content": "Future test",
                    "status": "complete",
                    "extraMeta": true
                }}
            ]
        }}"#
    );
    std::fs::write(
        conv_dir.join(format!("{valid_id}.json")),
        future_schema_json,
    )
    .unwrap();

    // 2. Write a corrupt/unparseable JSON file
    std::fs::write(
        conv_dir.join(format!("{corrupt_id}.json")),
        "{ invalid json content ...",
    )
    .unwrap();

    // Loading valid future-schema conversation succeeds
    let loaded = load_conversation_from_disk(&data_root, &valid_id).unwrap();
    assert_eq!(loaded.id, valid_id);
    assert_eq!(loaded.messages.len(), 1);

    // Loading corrupt conversation returns clean error
    let err = load_conversation_from_disk(&data_root, &corrupt_id);
    assert!(err.is_err());
    assert!(err.unwrap_err().contains("corrupted"));

    // Listing conversations gracefully skips corrupted files without failing
    let list = list_conversations_from_disk(&data_root).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, valid_id);
}

// ---------------------------------------------------------------------------
// 5. Bounded Inputs and Context Token Budget Tests
// ---------------------------------------------------------------------------

#[test]
fn test_history_context_bounding_cheap_char_budget() {
    let mut messages = Vec::new();
    for i in 0..10 {
        messages.push(ChatMessage {
            id: format!("msg-{i}"),
            role: if i % 2 == 0 {
                MessageRole::User
            } else {
                MessageRole::Assistant
            },
            content: format!("Message content number {i} - padding text."),
            status: MessageStatus::Complete,
        });
    }

    // Add a failed message in the middle that should be skipped
    messages.insert(
        5,
        ChatMessage {
            id: "failed-msg".to_string(),
            role: MessageRole::Assistant,
            content: "This was failed".to_string(),
            status: MessageStatus::Failed,
        },
    );

    // Limit budget to ~120 characters
    let bounded = prepare_bounded_messages(&messages, 120);

    // Verifications:
    // 1. Preserves chronological order
    for w in bounded.windows(2) {
        assert!(w[0].content < w[1].content);
    }
    // 2. Does not include failed messages
    assert!(!bounded.iter().any(|m| m.content == "This was failed"));
    // 3. Characters fit within budget
    let total_chars: usize = bounded.iter().map(|m| m.content.len()).sum();
    assert!(total_chars <= 120);
    // 4. Original messages list remains 100% unchanged
    assert_eq!(messages.len(), 11);
}

#[test]
fn test_input_bounds_constants() {
    assert_eq!(MAX_INPUT_CHARS, 32_000);
    assert_eq!(MAX_HISTORY_CHARS, 16_000);
}

// ---------------------------------------------------------------------------
// 6. Concurrency Guard & Cancellation Tests
// ---------------------------------------------------------------------------

#[test]
fn test_chat_state_prevents_overlapping_requests() {
    let store = MockSecretStore::new();
    let state = ChatState::new(store);

    let req1 = "req-1";
    let req2 = "req-2";
    let conv1 = uuid::Uuid::new_v4().to_string();

    // First request starts successfully
    let rx1 = state.try_start_request(req1, &conv1);
    assert!(rx1.is_ok());

    // Second request while first is active fails
    let rx2 = state.try_start_request(req2, &conv1);
    assert!(rx2.is_err());
    assert!(rx2.unwrap_err().contains("currently active"));

    // Cancel req1
    let cancelled = state.cancel_request(req1);
    assert!(cancelled);
    assert!(*rx1.unwrap().borrow());

    // Finish req1
    state.finish_request(req1);

    // Now req2 can start
    let rx2_retry = state.try_start_request(req2, &conv1);
    assert!(rx2_retry.is_ok());
}

// ---------------------------------------------------------------------------
// 7. Fixture Server Mock Tests (Model Discovery & Network Errors)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_fetch_models_against_fixture_server() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let body = r#"{"data":[{"id":"gpt-4o"},{"id":"claude-3-5-sonnet"}]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(resp.as_bytes()).await;
        }
    });

    let client = build_http_client().unwrap();
    let base_url = format!("http://{addr}/v1");
    let models = fetch_models(&client, &base_url, Some("mock-key"))
        .await
        .unwrap();

    assert_eq!(models, vec!["claude-3-5-sonnet", "gpt-4o"]);
}

#[tokio::test]
async fn test_fetch_models_sanitized_connection_refusal() {
    let client = build_http_client().unwrap();
    // Port 1 is not running anything
    let err = fetch_models(&client, "http://127.0.0.1:1/v1", None)
        .await
        .unwrap_err();
    assert!(err.contains("Failed to connect to 9router"));
}

// ---------------------------------------------------------------------------
// 8. End-to-End Fixture Stream Tests (Streaming, Cancellation, Errors, Save-Before-Network)
// ---------------------------------------------------------------------------

struct TestEventSink {
    events: std::sync::Mutex<Vec<super::models::ChatStreamPayload>>,
}

impl TestEventSink {
    fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            events: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn get_events(&self) -> Vec<super::models::ChatStreamPayload> {
        self.events.lock().unwrap().clone()
    }
}

impl super::commands::ChatEventSink for TestEventSink {
    fn emit_event(&self, payload: super::models::ChatStreamPayload) {
        self.events.lock().unwrap().push(payload);
    }
}

async fn read_http_request(socket: &mut tokio::net::TcpStream) -> String {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut temp = [0u8; 1024];
    let mut content_length = 0;
    let mut headers_done = false;

    while let Ok(n) = socket.read(&mut temp).await {
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&temp[..n]);
        if !headers_done {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                headers_done = true;
                let headers_str = String::from_utf8_lossy(&buf[..pos]);
                for line in headers_str.lines() {
                    let lower = line.to_ascii_lowercase();
                    if lower.starts_with("content-length:") {
                        let parts: Vec<&str> = line.splitn(2, ':').collect();
                        if parts.len() == 2 {
                            content_length = parts[1].trim().parse::<usize>().unwrap_or(0);
                        }
                    }
                }
                let body_read = buf.len() - (pos + 4);
                if body_read >= content_length {
                    break;
                }
            }
        } else {
            let pos = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
            let body_read = buf.len() - (pos + 4);
            if body_read >= content_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

#[tokio::test]
async fn test_execute_chat_stream_full_success() {
    let env = TestEnv::new("stream_success");
    let data_root = env.data_root();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "What is the capital of France?".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Capital of France".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    // Start mock streaming SSE server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let _ = read_http_request(&mut socket).await;

            let chunk1 = "data: {\"choices\":[{\"delta\":{\"content\":\"Paris\"}}]}\n\n";
            let chunk2 = "data: {\"choices\":[{\"delta\":{\"content\":\" is the capital.\"}}]}\n\n";
            let chunk3 = "data: [DONE]\n\n";

            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
            let _ = socket.write_all(header.as_bytes()).await;

            // Send chunked HTTP response
            for c in [chunk1, chunk2, chunk3] {
                let chunk_header = format!("{:X}\r\n{}\r\n", c.len(), c);
                let _ = socket.write_all(chunk_header.as_bytes()).await;
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-stream-1";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    super::commands::execute_chat_stream(
        sink.clone(),
        state.clone(),
        data_root.clone(),
        env.config_dir(),
        conv_id.clone(),
        req_id.to_string(),
        base_url,
        "test-model".to_string(),
        None,
        initial_conv.messages.clone(),
        cancel_rx,
    )
    .await;

    // 1. Verify stream events
    let events = sink.get_events();
    assert!(!events.is_empty());

    // Expect deltas "Paris" and " is the capital."
    let deltas: Vec<String> = events.iter().filter_map(|e| e.delta.clone()).collect();
    assert_eq!(
        deltas,
        vec!["Paris".to_string(), " is the capital.".to_string()]
    );

    // Expect exactly ONE terminal event with done: true and status: complete
    let terminal_events: Vec<&super::models::ChatStreamPayload> =
        events.iter().filter(|e| e.done == Some(true)).collect();
    assert_eq!(terminal_events.len(), 1);
    assert_eq!(terminal_events[0].status, Some(MessageStatus::Complete));
    assert_eq!(terminal_events[0].error, None);

    // 2. Verify persisted conversation on disk
    let loaded = load_conversation_from_disk(&data_root, &conv_id).unwrap();
    assert_eq!(loaded.messages.len(), 2);
    let assistant_msg = &loaded.messages[1];
    assert_eq!(assistant_msg.role, MessageRole::Assistant);
    assert_eq!(assistant_msg.content, "Paris is the capital.");
    assert_eq!(assistant_msg.status, MessageStatus::Complete);

    // 3. Verify state guard released
    assert!(state.try_start_request("req-new", &conv_id).is_ok());
}

#[tokio::test]
async fn test_execute_chat_stream_cancellation_mid_stream() {
    let env = TestEnv::new("stream_cancel");
    let data_root = env.data_root();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "Tell me a long story".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Story".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let _ = read_http_request(&mut socket).await;

            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
            let _ = socket.write_all(header.as_bytes()).await;

            let chunk = "data: {\"choices\":[{\"delta\":{\"content\":\"Once upon a time\"}}]}\n\n";
            let chunk_header = format!("{:X}\r\n{}\r\n", chunk.len(), chunk);
            let _ = socket.write_all(chunk_header.as_bytes()).await;

            // Wait without closing so client can cancel
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-cancel-1";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    // Spawn streamer
    let sink_clone = sink.clone();
    let state_clone = state.clone();
    let data_root_clone = data_root.clone();
    let config_dir_clone = env.config_dir();
    let conv_id_clone = conv_id.clone();
    let initial_messages = initial_conv.messages.clone();

    let handle = tokio::spawn(async move {
        super::commands::execute_chat_stream(
            sink_clone,
            state_clone,
            data_root_clone,
            config_dir_clone,
            conv_id_clone,
            req_id.to_string(),
            base_url,
            "test-model".to_string(),
            None,
            initial_messages,
            cancel_rx,
        )
        .await;
    });

    // Wait for the first delta to arrive
    for _ in 0..100 {
        if !sink.get_events().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // Trigger cancellation
    state.cancel_request(req_id);

    // Wait for streamer task to exit
    let _ = handle.await;

    // 1. Verify terminal event emitted with status: cancelled
    let events = sink.get_events();
    let terminal_events: Vec<&super::models::ChatStreamPayload> =
        events.iter().filter(|e| e.done == Some(true)).collect();
    assert_eq!(terminal_events.len(), 1);
    assert_eq!(terminal_events[0].status, Some(MessageStatus::Cancelled));

    // 2. Verify partial assistant message persisted to disk with status: cancelled
    let loaded = load_conversation_from_disk(&data_root, &conv_id).unwrap();
    assert_eq!(loaded.messages.len(), 2);
    let assistant_msg = &loaded.messages[1];
    assert_eq!(assistant_msg.role, MessageRole::Assistant);
    assert!(assistant_msg.content.contains("Once upon a time"));
    assert_eq!(assistant_msg.status, MessageStatus::Cancelled);

    // 3. Verify active request guard is freed
    assert!(
        state
            .try_start_request("req-after-cancel", &conv_id)
            .is_ok()
    );
}

#[tokio::test]
async fn test_execute_chat_stream_server_error_sanitization() {
    let env = TestEnv::new("stream_error");
    let data_root = env.data_root();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "Trigger error".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Error Test".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let _ = read_http_request(&mut socket).await;

            let body = "{\"error\":{\"message\":\"Internal failure with secret sql query\"}}";
            let resp = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(resp.as_bytes()).await;
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-err-1";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    super::commands::execute_chat_stream(
        sink.clone(),
        state.clone(),
        data_root.clone(),
        env.config_dir(),
        conv_id.clone(),
        req_id.to_string(),
        base_url,
        "test-model".to_string(),
        None,
        initial_conv.messages.clone(),
        cancel_rx,
    )
    .await;

    // 1. Verify terminal event emitted with status: failed and sanitized error
    let events = sink.get_events();
    let terminal_events: Vec<&super::models::ChatStreamPayload> =
        events.iter().filter(|e| e.done == Some(true)).collect();
    assert_eq!(terminal_events.len(), 1);
    assert_eq!(terminal_events[0].status, Some(MessageStatus::Failed));
    assert!(terminal_events[0].error.is_some());
    let err_str = terminal_events[0].error.as_ref().unwrap();
    // Sanitized: should not echo response body
    assert!(!err_str.contains("secret sql query"));
    assert!(err_str.contains("500"));

    // 2. Verify assistant saved to disk with status: failed
    let loaded = load_conversation_from_disk(&data_root, &conv_id).unwrap();
    assert_eq!(loaded.messages.len(), 2);
    let assistant_msg = &loaded.messages[1];
    assert_eq!(assistant_msg.status, MessageStatus::Failed);

    // 3. Verify active request guard is freed
    assert!(state.try_start_request("req-after-err", &conv_id).is_ok());
}

#[tokio::test]
async fn test_execute_chat_stream_finish_reason_without_done_succeeds() {
    let env = TestEnv::new("stream_finish_reason");
    let data_root = env.data_root();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "What is the capital of France?".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Capital of France".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    // Start mock streaming SSE server that sends finish_reason: "stop" without [DONE]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let _ = read_http_request(&mut socket).await;

            let chunk1 = "data: {\"choices\":[{\"delta\":{\"content\":\"Paris\"}}]}\n\n";
            let chunk2 = "data: {\"choices\":[{\"delta\":{\"content\":\" is the capital.\"},\"finish_reason\":\"stop\"}]}\n\n";

            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
            let _ = socket.write_all(header.as_bytes()).await;

            for c in [chunk1, chunk2] {
                let chunk_header = format!("{:X}\r\n{}\r\n", c.len(), c);
                let _ = socket.write_all(chunk_header.as_bytes()).await;
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            // Close stream without [DONE]
            let _ = socket.write_all(b"0\r\n\r\n").await;
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-stream-finish-reason";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    super::commands::execute_chat_stream(
        sink.clone(),
        state.clone(),
        data_root.clone(),
        env.config_dir(),
        conv_id.clone(),
        req_id.to_string(),
        base_url,
        "test-model".to_string(),
        None,
        initial_conv.messages.clone(),
        cancel_rx,
    )
    .await;

    // 1. Verify stream events
    let events = sink.get_events();
    assert!(!events.is_empty());

    let deltas: Vec<String> = events.iter().filter_map(|e| e.delta.clone()).collect();
    assert_eq!(
        deltas,
        vec!["Paris".to_string(), " is the capital.".to_string()]
    );

    let terminal_events: Vec<&super::models::ChatStreamPayload> =
        events.iter().filter(|e| e.done == Some(true)).collect();
    assert_eq!(terminal_events.len(), 1);
    assert_eq!(terminal_events[0].status, Some(MessageStatus::Complete));
    assert_eq!(terminal_events[0].error, None);

    // 2. Verify persisted conversation on disk
    let loaded = load_conversation_from_disk(&data_root, &conv_id).unwrap();
    assert_eq!(loaded.messages.len(), 2);
    let assistant_msg = &loaded.messages[1];
    assert_eq!(assistant_msg.role, MessageRole::Assistant);
    assert_eq!(assistant_msg.content, "Paris is the capital.");
    assert_eq!(assistant_msg.status, MessageStatus::Complete);
}

#[tokio::test]
async fn test_execute_chat_stream_premature_eof_fails() {
    let env = TestEnv::new("stream_premature_eof");
    let data_root = env.data_root();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "Hello".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Hello".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    // Mock server sends partial delta, then closes abruptly without finish_reason or [DONE]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let _ = read_http_request(&mut socket).await;

            let chunk1 =
                "data: {\"choices\":[{\"delta\":{\"content\":\"Incomplete thought...\"}}]}\n\n";

            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
            let _ = socket.write_all(header.as_bytes()).await;

            let chunk_header = format!("{:X}\r\n{}\r\n", chunk1.len(), chunk1);
            let _ = socket.write_all(chunk_header.as_bytes()).await;
            // Close stream immediately: premature EOF
            let _ = socket.write_all(b"0\r\n\r\n").await;
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-stream-premature-eof";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    super::commands::execute_chat_stream(
        sink.clone(),
        state.clone(),
        data_root.clone(),
        env.config_dir(),
        conv_id.clone(),
        req_id.to_string(),
        base_url,
        "test-model".to_string(),
        None,
        initial_conv.messages.clone(),
        cancel_rx,
    )
    .await;

    // 1. Verify partial delta was emitted
    let events = sink.get_events();
    let deltas: Vec<String> = events.iter().filter_map(|e| e.delta.clone()).collect();
    assert_eq!(deltas, vec!["Incomplete thought...".to_string()]);

    // 2. Verify premature EOF was NOT turned into success: marked Failed with specific error
    let terminal_events: Vec<&super::models::ChatStreamPayload> =
        events.iter().filter(|e| e.done == Some(true)).collect();
    assert_eq!(terminal_events.len(), 1);
    assert_eq!(terminal_events[0].status, Some(MessageStatus::Failed));
    assert_eq!(
        terminal_events[0].error,
        Some("Stream ended before completion was confirmed.".to_string())
    );

    // 3. Verify saved message on disk is marked Failed
    let loaded = load_conversation_from_disk(&data_root, &conv_id).unwrap();
    assert_eq!(loaded.messages.len(), 2);
    let assistant_msg = &loaded.messages[1];
    assert_eq!(assistant_msg.status, MessageStatus::Failed);
    assert_eq!(assistant_msg.content, "Incomplete thought...");
}

#[tokio::test]
async fn test_execute_chat_stream_finish_reason_with_done_succeeds() {
    let env = TestEnv::new("stream_finish_and_done");
    let data_root = env.data_root();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "Hello".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Hello".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let _ = read_http_request(&mut socket).await;

            let chunk1 = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello world\"}}]}\n\n";
            let chunk2 = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
            let chunk3 = "data: [DONE]\n\n";

            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
            let _ = socket.write_all(header.as_bytes()).await;

            for c in [chunk1, chunk2, chunk3] {
                let chunk_header = format!("{:X}\r\n{}\r\n", c.len(), c);
                let _ = socket.write_all(chunk_header.as_bytes()).await;
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-stream-finish-and-done";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    super::commands::execute_chat_stream(
        sink.clone(),
        state.clone(),
        data_root.clone(),
        env.config_dir(),
        conv_id.clone(),
        req_id.to_string(),
        base_url,
        "test-model".to_string(),
        None,
        initial_conv.messages.clone(),
        cancel_rx,
    )
    .await;

    let events = sink.get_events();
    let terminal_events: Vec<&super::models::ChatStreamPayload> =
        events.iter().filter(|e| e.done == Some(true)).collect();
    assert_eq!(terminal_events.len(), 1);
    assert_eq!(terminal_events[0].status, Some(MessageStatus::Complete));
    assert_eq!(terminal_events[0].error, None);

    let loaded = load_conversation_from_disk(&data_root, &conv_id).unwrap();
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[1].status, MessageStatus::Complete);
    assert_eq!(loaded.messages[1].content, "Hello world");
}

#[tokio::test]
async fn test_execute_chat_stream_explicit_error_in_stream_fails() {
    let env = TestEnv::new("stream_explicit_err");
    let data_root = env.data_root();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "Compute".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Compute".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let _ = read_http_request(&mut socket).await;

            let chunk1 = "data: {\"choices\":[{\"delta\":{\"content\":\"Partial answer\"}}]}\n\n";
            let chunk2 = "data: {\"error\":{\"message\":\"Upstream backend overloaded\"}}\n\n";

            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
            let _ = socket.write_all(header.as_bytes()).await;

            for c in [chunk1, chunk2] {
                let chunk_header = format!("{:X}\r\n{}\r\n", c.len(), c);
                let _ = socket.write_all(chunk_header.as_bytes()).await;
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-stream-explicit-err";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    super::commands::execute_chat_stream(
        sink.clone(),
        state.clone(),
        data_root.clone(),
        env.config_dir(),
        conv_id.clone(),
        req_id.to_string(),
        base_url,
        "test-model".to_string(),
        None,
        initial_conv.messages.clone(),
        cancel_rx,
    )
    .await;

    let events = sink.get_events();
    let terminal_events: Vec<&super::models::ChatStreamPayload> =
        events.iter().filter(|e| e.done == Some(true)).collect();
    assert_eq!(terminal_events.len(), 1);
    assert_eq!(terminal_events[0].status, Some(MessageStatus::Failed));
    assert!(terminal_events[0].error.is_some());
    assert!(
        terminal_events[0]
            .error
            .as_ref()
            .unwrap()
            .contains("Upstream backend overloaded")
    );

    let loaded = load_conversation_from_disk(&data_root, &conv_id).unwrap();
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[1].status, MessageStatus::Failed);
}

// ---------------------------------------------------------------------------
// 9. Conversation Deletion Backend Tests
// ---------------------------------------------------------------------------

#[test]
fn test_is_matching_conversation_file_logic() {
    let id = "e89b12d3-a456-4266-b456-426614174000";
    let other_id = "00000000-0000-0000-0000-000000000000";

    assert!(is_matching_conversation_file(&format!("{id}.json"), id));
    assert!(is_matching_conversation_file(&format!(".{id}.json"), id));
    assert!(is_matching_conversation_file(
        &format!(".{id}.bak_123_456"),
        id
    ));
    assert!(is_matching_conversation_file(
        &format!(".{id}.tmp_123_456"),
        id
    ));
    assert!(is_matching_conversation_file(&format!("{id}.bak"), id));
    assert!(is_matching_conversation_file(&format!("{id}.tmp"), id));
    assert!(is_matching_conversation_file(&format!("{id}.json.bak"), id));
    assert!(is_matching_conversation_file(&format!("{id}.json.tmp"), id));

    // Case insensitivity
    assert!(is_matching_conversation_file(
        &format!("{}.JSON", id.to_uppercase()),
        id
    ));

    // Non-matching
    assert!(!is_matching_conversation_file(
        &format!("{other_id}.json"),
        id
    ));
    assert!(!is_matching_conversation_file(
        &format!(".{other_id}.bak_1"),
        id
    ));
    assert!(!is_matching_conversation_file("chat_settings.json", id));
    assert!(!is_matching_conversation_file("settings.json", id));
    assert!(!is_matching_conversation_file("README.md", id));
    assert!(!is_matching_conversation_file(
        &format!("{id}_unrelated.txt"),
        id
    ));
    assert!(!is_matching_conversation_file(&format!("{id}.txt"), id));
}

#[test]
fn test_delete_conversation_invalid_id() {
    let env = TestEnv::new("delete_invalid_id");
    let data_root = env.data_root();

    let invalid_ids = [
        "",
        "not-a-uuid",
        "../traversal",
        "../../etc/passwd",
        "12345",
        "123e4567-e89b-12d3-a456-42661417400g",
        "123e4567-e89b-12d3-a456",
        "123e4567-e89b-12d3-a456-426614174000/extra",
    ];

    for bad_id in invalid_ids {
        let res = delete_conversation_from_disk(&data_root, bad_id);
        assert!(res.is_err(), "Expected error for invalid ID '{bad_id}'");
        let err = res.unwrap_err();
        assert!(
            err.contains("Invalid conversation ID"),
            "Expected 'Invalid conversation ID' error message, got: {err}"
        );
    }
}

#[test]
fn test_delete_conversation_correct_deletion() {
    let env = TestEnv::new("delete_correct");
    let data_root = env.data_root();

    let id = uuid::Uuid::new_v4().to_string();
    let conv = ConversationDetail {
        id: id.clone(),
        title: "To Be Deleted".to_string(),
        updated_at: "2026-10-19T12:00:00Z".to_string(),
        messages: vec![ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            content: "Please delete me".to_string(),
            status: MessageStatus::Complete,
        }],
        ..Default::default()
    };

    save_conversation_atomic(&data_root, &conv).unwrap();
    let conv_dir = get_conversations_dir(&data_root).unwrap();
    let primary_file = conv_dir.join(format!("{id}.json"));
    assert!(primary_file.exists());

    // Verify it loads and lists
    assert_eq!(load_conversation_from_disk(&data_root, &id).unwrap().id, id);
    assert_eq!(list_conversations_from_disk(&data_root).unwrap().len(), 1);

    // Perform deletion
    let del_res = delete_conversation_from_disk(&data_root, &id);
    assert!(del_res.is_ok(), "Deletion should succeed");

    // Primary file is gone
    assert!(!primary_file.exists());

    // Cannot load
    let load_err = load_conversation_from_disk(&data_root, &id);
    assert!(load_err.is_err());
    assert!(load_err.unwrap_err().contains("not found"));

    // Listing is empty
    assert!(list_conversations_from_disk(&data_root).unwrap().is_empty());
}

#[test]
fn test_delete_conversation_backup_and_temp_cleanup() {
    let env = TestEnv::new("delete_backup_cleanup");
    let data_root = env.data_root();
    let conv_dir = get_conversations_dir(&data_root).unwrap();

    let id = uuid::Uuid::new_v4().to_string();
    let conv = ConversationDetail {
        id: id.clone(),
        title: "Backup Cleanup Test".to_string(),
        updated_at: "2026-10-19T12:00:00Z".to_string(),
        messages: vec![ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            content: "Recoverable content".to_string(),
            status: MessageStatus::Complete,
        }],
        ..Default::default()
    };

    save_conversation_atomic(&data_root, &conv).unwrap();

    // Create lingering backup and temp files matching this ID
    let serialized = serde_json::to_string_pretty(&conv).unwrap();
    let bak1 = conv_dir.join(format!(".{id}.bak_1234_5678"));
    let bak2 = conv_dir.join(format!("{id}.bak"));
    let bak3 = conv_dir.join(format!("{id}.json.bak"));
    let tmp1 = conv_dir.join(format!(".{id}.tmp_1234_5678"));
    let tmp2 = conv_dir.join(format!("{id}.tmp"));
    let tmp3 = conv_dir.join(format!("{id}.json.tmp"));

    std::fs::write(&bak1, &serialized).unwrap();
    std::fs::write(&bak2, &serialized).unwrap();
    std::fs::write(&bak3, &serialized).unwrap();
    std::fs::write(&tmp1, b"partial tmp data").unwrap();
    std::fs::write(&tmp2, b"partial tmp data").unwrap();
    std::fs::write(&tmp3, b"partial tmp data").unwrap();

    // Verify lingering backup allows recovery if primary file is missing
    let primary_file = conv_dir.join(format!("{id}.json"));
    std::fs::remove_file(&primary_file).unwrap();
    let recovered = load_conversation_from_disk(&data_root, &id).unwrap();
    assert_eq!(recovered.id, id);

    // Re-create primary file
    std::fs::write(&primary_file, &serialized).unwrap();

    // Perform deletion
    delete_conversation_from_disk(&data_root, &id).unwrap();

    // Assert primary and ALL backup/temp files are gone
    assert!(!primary_file.exists());
    assert!(!bak1.exists());
    assert!(!bak2.exists());
    assert!(!bak3.exists());
    assert!(!tmp1.exists());
    assert!(!tmp2.exists());
    assert!(!tmp3.exists());

    // Verify conversation cannot resurrect from lingering backup
    let load_res = load_conversation_from_disk(&data_root, &id);
    assert!(load_res.is_err());
    assert!(load_res.unwrap_err().contains("not found"));
}

#[test]
fn test_delete_conversation_unrelated_preserved() {
    let env = TestEnv::new("delete_unrelated_preserved");
    let data_root = env.data_root();
    let conv_dir = get_conversations_dir(&data_root).unwrap();

    let id_a = uuid::Uuid::new_v4().to_string();
    let id_b = uuid::Uuid::new_v4().to_string();

    let conv_a = ConversationDetail {
        id: id_a.clone(),
        title: "Conversation A".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            content: "Hello A".to_string(),
            status: MessageStatus::Complete,
        }],
        ..Default::default()
    };

    let conv_b = ConversationDetail {
        id: id_b.clone(),
        title: "Conversation B".to_string(),
        updated_at: "2026-10-19T11:00:00Z".to_string(),
        messages: vec![ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            content: "Hello B".to_string(),
            status: MessageStatus::Complete,
        }],
        ..Default::default()
    };

    save_conversation_atomic(&data_root, &conv_a).unwrap();
    save_conversation_atomic(&data_root, &conv_b).unwrap();

    // Add backup for conversation B
    let bak_b = conv_dir.join(format!(".{id_b}.bak_9999_8888"));
    std::fs::write(&bak_b, serde_json::to_string(&conv_b).unwrap()).unwrap();

    // Add unrelated files in conversations dir
    let notes = conv_dir.join("notes.txt");
    std::fs::write(&notes, b"keep this safe").unwrap();

    // Add unrelated config files in data root
    let settings_file = data_root.join("settings.json");
    std::fs::write(&settings_file, b"{\"theme\":\"dark\"}").unwrap();

    // Delete conversation A
    delete_conversation_from_disk(&data_root, &id_a).unwrap();

    // Conversation A is deleted
    assert!(load_conversation_from_disk(&data_root, &id_a).is_err());

    // Conversation B is 100% intact
    let loaded_b = load_conversation_from_disk(&data_root, &id_b).unwrap();
    assert_eq!(loaded_b.id, id_b);
    assert_eq!(loaded_b.title, "Conversation B");
    assert!(bak_b.exists());

    // Unrelated files in conversations dir and data root are preserved
    assert!(notes.exists());
    assert_eq!(std::fs::read_to_string(&notes).unwrap(), "keep this safe");
    assert!(settings_file.exists());
    assert_eq!(
        std::fs::read_to_string(&settings_file).unwrap(),
        "{\"theme\":\"dark\"}"
    );

    // Listing still returns conversation B
    let list = list_conversations_from_disk(&data_root).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, id_b);
}

#[test]
fn test_delete_conversation_repeat_deletion_idempotent() {
    let env = TestEnv::new("delete_idempotent");
    let data_root = env.data_root();

    let valid_id = uuid::Uuid::new_v4().to_string();

    // Case 1: conversations directory does not even exist yet
    let res0 = delete_conversation_from_disk(&data_root, &valid_id);
    assert!(
        res0.is_ok(),
        "Must succeed idempotently when conv dir does not exist"
    );

    // Create conversations dir
    let _ = get_conversations_dir(&data_root).unwrap();

    // Case 2: ID never existed
    let res1 = delete_conversation_from_disk(&data_root, &valid_id);
    assert!(
        res1.is_ok(),
        "Must succeed idempotently when ID never existed"
    );

    // Case 3: Create, delete, then repeat deletion
    let conv = ConversationDetail {
        id: valid_id.clone(),
        title: "Idempotent".to_string(),
        updated_at: "2026-10-19T12:00:00Z".to_string(),
        messages: Vec::new(),
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &conv).unwrap();

    assert!(delete_conversation_from_disk(&data_root, &valid_id).is_ok());
    assert!(delete_conversation_from_disk(&data_root, &valid_id).is_ok());
    assert!(delete_conversation_from_disk(&data_root, &valid_id).is_ok());

    // Case 4: Non-existent data root should fail (requires valid data root)
    let non_existent_root = env.root.join("missing_dir");
    let res_err = delete_conversation_from_disk(&non_existent_root, &valid_id);
    assert!(res_err.is_err());
    assert!(
        res_err
            .unwrap_err()
            .contains("does not exist or is unavailable")
    );
}

#[test]
fn test_delete_conversation_busy_request_rejected() {
    let store = MockSecretStore::new();
    let state = ChatState::new(store);

    let conv_id = uuid::Uuid::new_v4().to_string();
    let req_id = "req-busy-check";

    // Start request
    let _rx = state.try_start_request(req_id, &conv_id).unwrap();

    // Attempting deletion under active guard must reject
    {
        let active_guard = state.inner.active.lock().unwrap();
        assert!(active_guard.is_some());
        let active = active_guard.as_ref().unwrap();
        let err = format!(
            "Cannot delete conversation while a chat request ('{}') is active.",
            active.request_id
        );
        assert!(err.contains("req-busy-check"));
    }

    // Finish request
    state.finish_request(req_id);

    // Active guard is now clear
    {
        let active_guard = state.inner.active.lock().unwrap();
        assert!(active_guard.is_none());
    }

    // Atomic lock exclusivity check: while delete holds the lock, try_start_request cannot start
    {
        let _guard = state.inner.active.lock().unwrap();
        let state_clone = state.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            state_clone.try_start_request("req-contended", "conv-x")
        });

        started_rx.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        drop(_guard);

        let result = handle.join().unwrap();
        assert!(result.is_ok());
        state.finish_request("req-contended");
    }
}

#[test]
fn test_delete_conversation_unsafe_link_policy() {
    let env = TestEnv::new("unsafe_link");
    let data_root = env.data_root();
    let outside_dir = env.root.join("outside_secret");
    std::fs::create_dir_all(&outside_dir).unwrap();
    let secret_file = outside_dir.join("secret_data.txt");
    std::fs::write(&secret_file, b"TOP_SECRET_CREDENTIALS").unwrap();

    let valid_id = uuid::Uuid::new_v4().to_string();

    // 1. Test symlinked / junction conversations directory
    let conv_dir = data_root.join("conversations");
    #[cfg(windows)]
    let junction_created = {
        let target_str = outside_dir.to_string_lossy();
        let link_str = conv_dir.to_string_lossy();
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", &link_str, &target_str])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    };
    #[cfg(unix)]
    let junction_created = std::os::unix::fs::symlink(&outside_dir, &conv_dir).is_ok();

    if junction_created {
        assert!(is_symlink_or_reparse(&conv_dir).unwrap());
        let res = delete_conversation_from_disk(&data_root, &valid_id);
        assert!(res.is_err(), "Must reject junction conversations directory");
        let err = res.unwrap_err();
        assert!(
            err.contains("symlink or reparse point") || err.contains("rejecting"),
            "Error should cite symlink or reparse point, got: {err}"
        );
        assert!(secret_file.exists());
        assert_eq!(
            std::fs::read_to_string(&secret_file).unwrap(),
            "TOP_SECRET_CREDENTIALS"
        );

        #[cfg(windows)]
        let _ = std::process::Command::new("cmd")
            .args(["/C", "rmdir", &conv_dir.to_string_lossy()])
            .output();
        #[cfg(unix)]
        let _ = std::fs::remove_file(&conv_dir);
    }

    // 2. Test symlinked conversation file
    let conv_dir = get_conversations_dir(&data_root).unwrap();
    let file_link = conv_dir.join(format!("{valid_id}.json"));

    #[cfg(windows)]
    let file_link_created = std::os::windows::fs::symlink_file(&secret_file, &file_link).is_ok();
    #[cfg(unix)]
    let file_link_created = std::os::unix::fs::symlink(&secret_file, &file_link).is_ok();

    if file_link_created {
        assert!(is_symlink_or_reparse(&file_link).unwrap());
        let res = delete_conversation_from_disk(&data_root, &valid_id);
        assert!(res.is_err(), "Must reject symlinked conversation file");
        let err = res.unwrap_err();
        assert!(
            err.contains("symlink or reparse point") || err.contains("rejecting"),
            "Error should cite symlink or reparse point, got: {err}"
        );
        assert!(secret_file.exists());
        assert_eq!(
            std::fs::read_to_string(&secret_file).unwrap(),
            "TOP_SECRET_CREDENTIALS"
        );
    }
}

#[test]
fn test_delete_conversation_reports_partial_filesystem_errors() {
    let env = TestEnv::new("delete_partial_fs_err");
    let data_root = env.data_root();
    let conv_dir = get_conversations_dir(&data_root).unwrap();

    let id = uuid::Uuid::new_v4().to_string();
    let conv = ConversationDetail {
        id: id.clone(),
        title: "Partial Error Test".to_string(),
        updated_at: "2026-10-19T12:00:00Z".to_string(),
        messages: Vec::new(),
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &conv).unwrap();

    let bak_file = conv_dir.join(format!(".{id}.bak_lock_test"));
    std::fs::write(&bak_file, b"backup data").unwrap();

    #[cfg(windows)]
    let _locked_file = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&bak_file)
            .ok()
    };

    let res = delete_conversation_from_disk(&data_root, &id);

    #[cfg(windows)]
    {
        if _locked_file.is_some() {
            assert!(res.is_err(), "Must report partial filesystem error");
            let err = res.unwrap_err();
            assert!(
                err.contains("Errors encountered") || err.contains("Failed to delete"),
                "Error should report failed deletion, got: {err}"
            );
        }
    }

    #[cfg(windows)]
    drop(_locked_file);
}

// ---------------------------------------------------------------------------
// 10. Native Tool Calling Integration & Unit Tests
// ---------------------------------------------------------------------------

#[test]
fn test_tool_schema_and_constants() {
    let tools = get_chat_tools();
    assert_eq!(tools.len(), 6);
    let names: Vec<&str> = tools.iter().map(|t| t.function.name.as_str()).collect();
    assert!(names.contains(&"discover_game_projects"));
    assert!(names.contains(&"list_shareable_projects"));
    assert!(names.contains(&"list_project_directory"));
    assert!(names.contains(&"read_file_chunk"));
    assert!(names.contains(&"search_project_files"));
    assert!(names.contains(&"search_project_text"));

    for t in &tools {
        assert_eq!(t.tool_type, "function");
        assert!(!t.function.description.is_empty());
        assert!(t.function.parameters.is_object());
    }

    assert_eq!(MAX_TOOL_ROUNDS, 5);
    assert_eq!(MAX_TOOLS_PER_ROUND, 8);
    assert_eq!(MAX_TOTAL_TOOL_CALLS, 16);
    assert_eq!(MAX_TOOL_RESULT_BYTES, 64 * 1024);
    assert_eq!(MAX_CUMULATIVE_TOOL_OUTPUT_BYTES, 256 * 1024);
    assert_eq!(MAX_TOOL_ARGUMENTS_BYTES, 64 * 1024);
}

#[test]
fn test_tool_accumulator_split_arguments_and_bounds() {
    let mut acc = ToolCallAccumulator::new();

    // Chunk 1: index 0, starts function call
    acc.process_delta(
        0,
        Some("call_1".to_string()),
        Some("read_file_chunk".to_string()),
        Some("{\"path\":".to_string()),
    )
    .unwrap();

    // Chunk 2: index 0, continuation of arguments
    acc.process_delta(
        0,
        None,
        None,
        Some("\"C:\\\\test\\\\file.txt\"}".to_string()),
    )
    .unwrap();

    let calls = acc.into_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].name, "read_file_chunk");
    assert_eq!(calls[0].arguments, "{\"path\":\"C:\\\\test\\\\file.txt\"}");

    // Test argument size limit
    let mut acc_big = ToolCallAccumulator::new();
    acc_big
        .process_delta(
            0,
            Some("call_big".to_string()),
            Some("test".to_string()),
            Some("a".repeat(MAX_TOOL_ARGUMENTS_BYTES + 10)),
        )
        .expect_err("Must reject oversized arguments");

    // Test max tools per round limit
    let mut acc_many = ToolCallAccumulator::new();
    for i in 0..MAX_TOOLS_PER_ROUND {
        acc_many
            .process_delta(
                i,
                Some(format!("call_{i}")),
                Some("test".to_string()),
                Some("{}".to_string()),
            )
            .unwrap();
    }
    // Exceeding limit
    acc_many
        .process_delta(
            MAX_TOOLS_PER_ROUND,
            Some("overflow".to_string()),
            Some("test".to_string()),
            Some("{}".to_string()),
        )
        .expect_err("Must reject more than MAX_TOOLS_PER_ROUND calls");
}

#[test]
fn test_execute_tool_unit_behavior() {
    let env = TestEnv::new("tool_exec_unit");
    let cfg = env.config_dir();
    let proj = env.new_dir("UnitProject");
    let file = proj.join("test.txt");
    std::fs::write(&file, b"Hello tool world").unwrap();
    let canon_proj = dunce::canonicalize(&proj)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let canon_file = dunce::canonicalize(&file)
        .unwrap()
        .to_string_lossy()
        .to_string();

    // 1. Unknown tool
    let res = execute_tool(&cfg, "shell_exec", "{}");
    assert!(res.contains("Unknown tool 'shell_exec'."));

    // 2. Malformed arguments JSON
    let res = execute_tool(&cfg, "read_file_chunk", "{invalid json}");
    assert!(res.contains("Invalid arguments"));

    // 3. list_shareable_projects: initially empty
    let res = execute_tool(&cfg, "list_shareable_projects", "{}");
    assert_eq!(res, "{\"projects\":[]}");

    // Enable sharing
    add_project_root_path(&cfg, &proj).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_proj, Some(true)).unwrap();

    let res = execute_tool(&cfg, "list_shareable_projects", "{}");
    assert!(res.contains("UnitProject"));

    // 4. list_project_directory
    let list_args = serde_json::to_string(&serde_json::json!({
        "project_root": canon_proj,
    }))
    .unwrap();
    let res = execute_tool(&cfg, "list_project_directory", &list_args);
    assert!(res.contains("test.txt"));

    // 5. read_file_chunk on granted file
    let read_args = serde_json::to_string(&serde_json::json!({
        "path": canon_file,
    }))
    .unwrap();
    let res = execute_tool(&cfg, "read_file_chunk", &read_args);
    assert!(res.contains("Hello tool world"));

    // 6. read_file_chunk on sensitive file
    let env_file = proj.join(".env");
    std::fs::write(&env_file, b"KEY=secret").unwrap();
    let canon_env = dunce::canonicalize(&env_file)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let read_env_args = serde_json::to_string(&serde_json::json!({
        "path": canon_env,
    }))
    .unwrap();
    let res = execute_tool(&cfg, "read_file_chunk", &read_env_args);
    assert!(res.contains("Access to restricted folder or file is blocked"));

    // 7. read_file_chunk on ungranted outside file
    let outside = env.new_dir("outside");
    let out_file = outside.join("out.txt");
    std::fs::write(&out_file, b"outside").unwrap();
    let canon_out = dunce::canonicalize(&out_file)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let read_out_args = serde_json::to_string(&serde_json::json!({
        "path": canon_out,
    }))
    .unwrap();
    let res = execute_tool(&cfg, "read_file_chunk", &read_out_args);
    assert!(res.contains("File is outside approved project folders"));
}

#[test]
fn test_reauthorize_tool_result_unit_behavior() {
    let env = TestEnv::new("tool_reauth_unit");
    let cfg = env.config_dir();
    let proj = env.new_dir("ReauthProject");
    let file = proj.join("code.rs");
    std::fs::write(&file, b"fn code() {}").unwrap();
    let canon_proj = dunce::canonicalize(&proj)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let canon_file = dunce::canonicalize(&file)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &proj).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_proj, Some(true)).unwrap();

    let args = serde_json::to_string(&serde_json::json!({
        "path": canon_file,
    }))
    .unwrap();

    let mut content = "{\"text\":\"fn code() {}\"}".to_string();

    // While grant is active, reauthorization leaves content intact
    reauthorize_tool_result(&cfg, "read_file_chunk", &args, &mut content);
    assert_eq!(content, "{\"text\":\"fn code() {}\"}");

    // Revoke provider sharing
    toggle_project_provider_sharing_impl(&cfg, &canon_proj, Some(false)).unwrap();

    // Reauthorization must purge content!
    reauthorize_tool_result(&cfg, "read_file_chunk", &args, &mut content);
    assert!(content.contains("Provider sharing is not enabled for this project folder"));
    assert!(!content.contains("fn code() {}"));
}

#[tokio::test]
async fn test_execute_chat_stream_tool_calling_flow() {
    let env = TestEnv::new("stream_tool_flow");
    let data_root = env.data_root();
    let cfg = env.config_dir();

    // Setup granted project folder with a file
    let proj = env.new_dir("AppProject");
    let notes = proj.join("notes.txt");
    std::fs::write(&notes, b"Alpha Bravo Charlie").unwrap();
    let canon_proj = dunce::canonicalize(&proj)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let canon_notes = dunce::canonicalize(&notes)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &proj).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_proj, Some(true)).unwrap();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "Please read notes.txt".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Tool Test".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let canon_notes_clone = canon_notes.clone();

    tokio::spawn(async move {
        let mut request_count = 0;
        while let Ok((mut socket, _)) = listener.accept().await {
            request_count += 1;
            let req_text = read_http_request(&mut socket).await;

            if request_count == 1 {
                // Round 1: Model requests tool call to read_file_chunk with split arguments
                let full_args = serde_json::to_string(&serde_json::json!({
                    "path": canon_notes_clone
                }))
                .unwrap();
                let split_pt = full_args.len() / 2;
                let arg_part1 = &full_args[..split_pt];
                let arg_part2 = &full_args[split_pt..];

                let sse_obj1 = serde_json::json!({
                    "choices": [{
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "id": "call_read_1",
                                "type": "function",
                                "function": {
                                    "name": "read_file_chunk",
                                    "arguments": arg_part1
                                }
                            }]
                        },
                        "finish_reason": null
                    }]
                });
                let sse_obj2 = serde_json::json!({
                    "choices": [{
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "function": {
                                    "arguments": arg_part2
                                }
                            }]
                        },
                        "finish_reason": "tool_calls"
                    }]
                });
                let part1 = format!("data: {}\n\n", serde_json::to_string(&sse_obj1).unwrap());
                let part2 = format!("data: {}\n\n", serde_json::to_string(&sse_obj2).unwrap());

                let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n";
                let _ = socket.write_all(header.as_bytes()).await;

                for p in [part1.as_str(), part2.as_str()] {
                    let chunk_hdr = format!("{:X}\r\n{}\r\n", p.len(), p);
                    let _ = socket.write_all(chunk_hdr.as_bytes()).await;
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                let _ = socket.write_all(b"0\r\n\r\n").await;
            } else if request_count == 2 {
                // Round 2: Verify request contains tool role message with the file chunk
                assert!(req_text.contains("\"role\":\"tool\""));
                assert!(req_text.contains("Alpha Bravo Charlie"));

                // Return final answer
                let ans = "data: {\"choices\":[{\"delta\":{\"content\":\"Notes say Alpha Bravo Charlie\"}}]}\n\n";
                let stop = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
                let done = "data: [DONE]\n\n";

                let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n";
                let _ = socket.write_all(header.as_bytes()).await;

                for p in [ans, stop, done] {
                    let chunk_hdr = format!("{:X}\r\n{}\r\n", p.len(), p);
                    let _ = socket.write_all(chunk_hdr.as_bytes()).await;
                }
                let _ = socket.write_all(b"0\r\n\r\n").await;
                break;
            }
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-tool-stream";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    super::commands::execute_chat_stream(
        sink.clone(),
        state.clone(),
        data_root.clone(),
        cfg.clone(),
        conv_id.clone(),
        req_id.to_string(),
        base_url,
        "test-model".to_string(),
        None,
        initial_conv.messages.clone(),
        cancel_rx,
    )
    .await;

    // Verify stream events
    let events = sink.get_events();
    let deltas: Vec<String> = events.iter().filter_map(|e| e.delta.clone()).collect();
    assert_eq!(deltas, vec!["Notes say Alpha Bravo Charlie".to_string()]);

    let terminal_events: Vec<&super::models::ChatStreamPayload> =
        events.iter().filter(|e| e.done == Some(true)).collect();
    assert_eq!(terminal_events.len(), 1);
    assert_eq!(terminal_events[0].status, Some(MessageStatus::Complete));
    assert_eq!(terminal_events[0].error, None);

    // Verify history non-persistence: disk must ONLY have user message and final assistant message!
    let loaded = load_conversation_from_disk(&data_root, &conv_id).unwrap();
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].role, MessageRole::User);
    assert_eq!(loaded.messages[1].role, MessageRole::Assistant);
    assert_eq!(loaded.messages[1].content, "Notes say Alpha Bravo Charlie");
    assert_eq!(loaded.messages[1].status, MessageStatus::Complete);
}

#[tokio::test]
async fn test_execute_chat_stream_tool_revocation_before_send() {
    let env = TestEnv::new("stream_revocation");
    let data_root = env.data_root();
    let cfg = env.config_dir();

    let proj = env.new_dir("RevokeProject");
    let file = proj.join("secret_data.txt");
    std::fs::write(&file, b"CLASSIFIED_INFO").unwrap();
    let canon_proj = dunce::canonicalize(&proj)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let canon_file = dunce::canonicalize(&file)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &proj).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_proj, Some(true)).unwrap();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "Read secret_data.txt".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Revoke Test".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let canon_file_clone = canon_file.clone();
    let cfg_clone = cfg.clone();
    let canon_proj_clone = canon_proj.clone();

    tokio::spawn(async move {
        let mut request_count = 0;
        while let Ok((mut socket, _)) = listener.accept().await {
            request_count += 1;
            let req_text = read_http_request(&mut socket).await;

            if request_count == 1 {
                // Revoke provider sharing BEFORE next round send boundary
                toggle_project_provider_sharing_impl(&cfg_clone, &canon_proj_clone, Some(false))
                    .unwrap();

                let full_args = serde_json::to_string(&serde_json::json!({
                    "path": canon_file_clone
                }))
                .unwrap();
                let sse_obj = serde_json::json!({
                    "choices": [{
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "id": "call_rev_1",
                                "type": "function",
                                "function": {
                                    "name": "read_file_chunk",
                                    "arguments": full_args
                                }
                            }]
                        },
                        "finish_reason": "tool_calls"
                    }]
                });
                let tool_call = format!("data: {}\n\n", serde_json::to_string(&sse_obj).unwrap());

                let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n";
                let _ = socket.write_all(header.as_bytes()).await;

                let chunk_hdr = format!("{:X}\r\n{}\r\n", tool_call.len(), tool_call);
                let _ = socket.write_all(chunk_hdr.as_bytes()).await;
                let _ = socket.write_all(b"0\r\n\r\n").await;
            } else if request_count == 2 {
                // Verify revocation prevented content from crossing send boundary!
                assert!(
                    !req_text.contains("CLASSIFIED_INFO"),
                    "Classified content must NOT cross network boundary"
                );
                assert!(
                    req_text.contains("Provider sharing is not enabled for this project folder")
                );

                let ans = "data: {\"choices\":[{\"delta\":{\"content\":\"Access revoked.\"}}]}\n\n";
                let stop = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
                let done = "data: [DONE]\n\n";

                let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n";
                let _ = socket.write_all(header.as_bytes()).await;

                for p in [ans, stop, done] {
                    let chunk_hdr = format!("{:X}\r\n{}\r\n", p.len(), p);
                    let _ = socket.write_all(chunk_hdr.as_bytes()).await;
                }
                let _ = socket.write_all(b"0\r\n\r\n").await;
                break;
            }
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-revoke-stream";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    super::commands::execute_chat_stream(
        sink.clone(),
        state.clone(),
        data_root.clone(),
        cfg.clone(),
        conv_id.clone(),
        req_id.to_string(),
        base_url,
        "test-model".to_string(),
        None,
        initial_conv.messages.clone(),
        cancel_rx,
    )
    .await;

    let events = sink.get_events();
    let deltas: Vec<String> = events.iter().filter_map(|e| e.delta.clone()).collect();
    assert_eq!(deltas, vec!["Access revoked.".to_string()]);
}

#[tokio::test]
async fn test_execute_chat_stream_tool_rounds_budget_exceeded() {
    let env = TestEnv::new("stream_rounds_budget");
    let data_root = env.data_root();
    let cfg = env.config_dir();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "Infinite loop tool caller".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Budget Test".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        // Always respond with a tool call
        while let Ok((mut socket, _)) = listener.accept().await {
            let _ = read_http_request(&mut socket).await;

            let tool_call = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_loop\",\"type\":\"function\",\"function\":{\"name\":\"list_shareable_projects\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n";
            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n";
            let _ = socket.write_all(header.as_bytes()).await;

            let chunk_hdr = format!("{:X}\r\n{}\r\n", tool_call.len(), tool_call);
            let _ = socket.write_all(chunk_hdr.as_bytes()).await;
            let _ = socket.write_all(b"0\r\n\r\n").await;
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-rounds-budget";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    super::commands::execute_chat_stream(
        sink.clone(),
        state.clone(),
        data_root.clone(),
        cfg.clone(),
        conv_id.clone(),
        req_id.to_string(),
        base_url,
        "test-model".to_string(),
        None,
        initial_conv.messages.clone(),
        cancel_rx,
    )
    .await;

    let events = sink.get_events();
    let terminal_events: Vec<&super::models::ChatStreamPayload> =
        events.iter().filter(|e| e.done == Some(true)).collect();
    assert_eq!(terminal_events.len(), 1);
    assert_eq!(terminal_events[0].status, Some(MessageStatus::Failed));
    let err = terminal_events[0].error.as_ref().unwrap();
    assert!(err.contains("Exceeded maximum tool calling rounds limit"));

    // Active lock freed
    assert!(
        state
            .try_start_request("req-after-budget", &conv_id)
            .is_ok()
    );
}

#[tokio::test]
async fn test_execute_chat_stream_cancellation_during_tool_round() {
    let env = TestEnv::new("stream_cancel_tool");
    let data_root = env.data_root();
    let cfg = env.config_dir();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "Cancel during tool".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Cancel Tool".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let _ = read_http_request(&mut socket).await;

            let tool_call = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_cancel\",\"type\":\"function\",\"function\":{\"name\":\"list_shareable_projects\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n";
            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n";
            let _ = socket.write_all(header.as_bytes()).await;

            let chunk_hdr = format!("{:X}\r\n{}\r\n", tool_call.len(), tool_call);
            let _ = socket.write_all(chunk_hdr.as_bytes()).await;
            let _ = socket.write_all(b"0\r\n\r\n").await;
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-cancel-tool";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    // Cancel before or immediately as stream begins
    state.cancel_request(req_id);

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    super::commands::execute_chat_stream(
        sink.clone(),
        state.clone(),
        data_root.clone(),
        cfg.clone(),
        conv_id.clone(),
        req_id.to_string(),
        base_url,
        "test-model".to_string(),
        None,
        initial_conv.messages.clone(),
        cancel_rx,
    )
    .await;

    let events = sink.get_events();
    let terminal_events: Vec<&super::models::ChatStreamPayload> =
        events.iter().filter(|e| e.done == Some(true)).collect();
    assert_eq!(terminal_events.len(), 1);
    assert_eq!(terminal_events[0].status, Some(MessageStatus::Cancelled));

    let loaded = load_conversation_from_disk(&data_root, &conv_id).unwrap();
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[1].status, MessageStatus::Cancelled);
}

#[tokio::test]
async fn test_execute_chat_stream_tool_denied_traversal_and_malformed() {
    let env = TestEnv::new("stream_denied_tool");
    let data_root = env.data_root();
    let cfg = env.config_dir();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "Read traversal file".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Denied Tool".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let mut request_count = 0;
        while let Ok((mut socket, _)) = listener.accept().await {
            request_count += 1;
            let req_text = read_http_request(&mut socket).await;

            if request_count == 1 {
                // Model tries path traversal
                let sse_obj = serde_json::json!({
                    "choices": [{
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "id": "call_trav_1",
                                "type": "function",
                                "function": {
                                    "name": "read_file_chunk",
                                    "arguments": "{\"path\":\"../../etc/shadow\"}"
                                }
                            }]
                        },
                        "finish_reason": "tool_calls"
                    }]
                });
                let tool_call = format!("data: {}\n\n", serde_json::to_string(&sse_obj).unwrap());

                let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n";
                let _ = socket.write_all(header.as_bytes()).await;

                let chunk_hdr = format!("{:X}\r\n{}\r\n", tool_call.len(), tool_call);
                let _ = socket.write_all(chunk_hdr.as_bytes()).await;
                let _ = socket.write_all(b"0\r\n\r\n").await;
            } else if request_count == 2 {
                // Verify request contains the authorization error
                assert!(req_text.contains("\"role\":\"tool\""));
                assert!(
                    req_text.contains("outside approved project folders")
                        || req_text.contains("blocked")
                        || req_text.contains("absolute")
                );

                let ans =
                    "data: {\"choices\":[{\"delta\":{\"content\":\"Access was denied.\"}}]}\n\n";
                let stop = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
                let done = "data: [DONE]\n\n";

                let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n";
                let _ = socket.write_all(header.as_bytes()).await;

                for p in [ans, stop, done] {
                    let chunk_hdr = format!("{:X}\r\n{}\r\n", p.len(), p);
                    let _ = socket.write_all(chunk_hdr.as_bytes()).await;
                }
                let _ = socket.write_all(b"0\r\n\r\n").await;
                break;
            }
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-denied-tool";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    super::commands::execute_chat_stream(
        sink.clone(),
        state.clone(),
        data_root.clone(),
        cfg.clone(),
        conv_id.clone(),
        req_id.to_string(),
        base_url,
        "test-model".to_string(),
        None,
        initial_conv.messages.clone(),
        cancel_rx,
    )
    .await;

    let events = sink.get_events();
    let deltas: Vec<String> = events.iter().filter_map(|e| e.delta.clone()).collect();
    assert_eq!(deltas, vec!["Access was denied.".to_string()]);
}

#[test]
fn tool_default_chunk_fits_serialized_result_budget() {
    let env = TestEnv::new("escaped_chunk_budget");
    let cfg = env.config_dir();
    let proj = env.new_dir("BudgetProject");
    let file = proj.join("large.txt");
    std::fs::write(&file, "\t".repeat(100_000)).unwrap();
    add_project_root_path(&cfg, &proj).unwrap();
    let root = dunce::canonicalize(&proj)
        .unwrap()
        .to_string_lossy()
        .to_string();
    toggle_project_provider_sharing_impl(&cfg, &root, Some(true)).unwrap();
    let args = serde_json::json!({"path": file.to_string_lossy()}).to_string();
    let result = execute_tool(&cfg, "read_file_chunk", &args);
    assert!(result.len() < super::tools::MAX_TOOL_RESULT_BYTES);
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["nextOffset"], 8192);
    assert_eq!(result["eof"], false);
}

// ---------------------------------------------------------------------------
// Tool Activity and Observable Stream Tests
// ---------------------------------------------------------------------------

#[test]
fn test_tool_activity_payload_contract() {
    let running = ToolActivityPayload {
        id: "call_abc".to_string(),
        name: "discover_game_projects".to_string(),
        status: ToolActivityStatus::Running,
        summary: Some("Discovering game projects...".to_string()),
    };
    let json_running = serde_json::to_string(&running).unwrap();
    assert!(json_running.contains("\"id\":\"call_abc\""));
    assert!(json_running.contains("\"name\":\"discover_game_projects\""));
    assert!(json_running.contains("\"status\":\"running\""));
    assert!(json_running.contains("\"summary\":\"Discovering game projects...\""));

    let succeeded = ToolActivityPayload {
        id: "call_abc".to_string(),
        name: "discover_game_projects".to_string(),
        status: ToolActivityStatus::Succeeded,
        summary: Some("Discovered 3 game projects".to_string()),
    };
    let json_succeeded = serde_json::to_string(&succeeded).unwrap();
    assert!(json_succeeded.contains("\"status\":\"succeeded\""));

    let failed = ToolActivityPayload {
        id: "call_abc".to_string(),
        name: "read_file_chunk".to_string(),
        status: ToolActivityStatus::Failed,
        summary: Some("Failed: Access denied".to_string()),
    };
    let json_failed = serde_json::to_string(&failed).unwrap();
    assert!(json_failed.contains("\"status\":\"failed\""));

    // Verify ChatStreamPayload embedding
    let stream_payload = super::models::ChatStreamPayload::activity(
        "req-1".to_string(),
        "conv-1".to_string(),
        succeeded,
    );
    let json_stream = serde_json::to_string(&stream_payload).unwrap();
    assert!(json_stream.contains("\"toolActivity\":{"));
    assert!(!json_stream.contains("\"done\":"));
}

#[test]
fn test_safe_running_summary_and_summarize_tool_output() {
    // 1. Safe running summary tests (no raw args, secrets or raw paths)
    assert_eq!(
        safe_running_summary("discover_game_projects"),
        "Discovering game projects in approved folders"
    );
    assert_eq!(
        safe_running_summary("list_shareable_projects"),
        "Checking shareable project roots"
    );
    assert_eq!(
        safe_running_summary("list_project_directory"),
        "Inspecting project directory"
    );
    assert_eq!(
        safe_running_summary("read_file_chunk"),
        "Reading file chunk"
    );
    assert_eq!(
        safe_running_summary("search_project_files"),
        "Searching project files"
    );
    assert_eq!(
        safe_running_summary("search_project_text"),
        "Searching project text"
    );
    assert_eq!(
        safe_running_summary("unknown_tool"),
        "Executing unknown_tool"
    );

    // 2. Summarize tool output tests
    // Discovery success
    let disc_success = serde_json::json!({
        "projects": [{"name": "A"}, {"name": "B"}],
        "coverage": "complete"
    })
    .to_string();
    let (status, summary) = summarize_tool_output("discover_game_projects", &disc_success);
    assert_eq!(status, ToolActivityStatus::Succeeded);
    assert_eq!(summary, "Found 2 game project(s)");

    // Search files success
    let files_success = serde_json::json!({
        "matches": [{"name": "Player.cs"}, {"name": "Enemy.cs"}],
        "coverage": "complete"
    })
    .to_string();
    let (status, summary) = summarize_tool_output("search_project_files", &files_success);
    assert_eq!(status, ToolActivityStatus::Succeeded);
    assert_eq!(summary, "Found 2 matching file(s)");

    // Search files partial
    let files_partial = serde_json::json!({
        "matches": [{"name": "Player.cs"}],
        "coverage": "partial"
    })
    .to_string();
    let (status, summary) = summarize_tool_output("search_project_files", &files_partial);
    assert_eq!(status, ToolActivityStatus::Succeeded);
    assert_eq!(summary, "Found 1 matching file(s) (partial search)");

    // Search text success
    let text_success = serde_json::json!({
        "matches": [{"path": "Player.cs", "snippet": "health"}],
        "filesScanned": 5,
        "coverage": "complete"
    })
    .to_string();
    let (status, summary) = summarize_tool_output("search_project_text", &text_success);
    assert_eq!(status, ToolActivityStatus::Succeeded);
    assert_eq!(summary, "Found 1 text match(es) across 5 file(s)");

    // Search text partial
    let text_partial = serde_json::json!({
        "matches": [{"path": "Player.cs", "snippet": "health"}],
        "filesScanned": 5,
        "coverage": "partial"
    })
    .to_string();
    let (status, summary) = summarize_tool_output("search_project_text", &text_partial);
    assert_eq!(status, ToolActivityStatus::Succeeded);
    assert_eq!(
        summary,
        "Found 1 text match(es) across 5 file(s) (partial search)"
    );

    // Discovery structured error
    let disc_err = serde_json::json!({
        "error": "Access denied: Root directory is not approved."
    })
    .to_string();
    let (status, summary) = summarize_tool_output("discover_game_projects", &disc_err);
    assert_eq!(status, ToolActivityStatus::Failed);
    assert_eq!(summary, "Access denied or restricted path");

    // File chunk read success
    let read_success = serde_json::json!({
        "text": "sample text",
        "bytesRead": 11,
        "eof": true
    })
    .to_string();
    let (status, summary) = summarize_tool_output("read_file_chunk", &read_success);
    assert_eq!(status, ToolActivityStatus::Succeeded);
    assert_eq!(summary, "Read 11 bytes (reached EOF)");

    // File chunk read error
    let read_err = serde_json::json!({
        "error": "File is outside approved project folders"
    })
    .to_string();
    let (status, summary) = summarize_tool_output("read_file_chunk", &read_err);
    assert_eq!(status, ToolActivityStatus::Failed);
    assert_eq!(summary, "Access denied or restricted path");
}

#[test]
fn test_execute_tool_discover_game_projects() {
    let env = TestEnv::new("tool_exec_discovery");
    let cfg = env.config_dir();
    let root = env.new_dir("DiscoveryTestRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    let proj = root.join("GodotGame");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join("project.godot"),
        b"config_version=5\n[application]\nconfig/name=\"Nebula\"\n",
    )
    .unwrap();

    let output = execute_tool(&cfg, TOOL_DISCOVER_GAME_PROJECTS, "{}");
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(parsed["coverage"], "complete");
    assert_eq!(parsed["truncated"], false);
    let projects = parsed["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["name"], "Nebula");
    assert_eq!(projects[0]["engine"], "Godot");
}

#[test]
fn test_reauthorize_tool_result_discovery() {
    let env = TestEnv::new("reauth_tool_discovery");
    let cfg = env.config_dir();
    let root = env.new_dir("ReauthDiscoveryRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    let proj = root.join("UnityGame");
    std::fs::create_dir_all(proj.join("ProjectSettings")).unwrap();
    std::fs::write(
        proj.join("ProjectSettings").join("ProjectVersion.txt"),
        b"m_EditorVersion: 2022.3.0f1\n",
    )
    .unwrap();
    std::fs::create_dir_all(proj.join("Assets")).unwrap();

    let initial_output = execute_tool(&cfg, TOOL_DISCOVER_GAME_PROJECTS, "{}");
    assert!(initial_output.contains("UnityGame"));

    let mut output_to_reauth = initial_output.clone();
    reauthorize_tool_result(
        &cfg,
        TOOL_DISCOVER_GAME_PROJECTS,
        "{}",
        &mut output_to_reauth,
    );
    assert_eq!(initial_output, output_to_reauth);

    // Revoke root
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(false)).unwrap();
    reauthorize_tool_result(
        &cfg,
        TOOL_DISCOVER_GAME_PROJECTS,
        "{}",
        &mut output_to_reauth,
    );
    assert!(
        output_to_reauth.contains("Provider sharing was revoked for all scanned project roots")
    );
    assert!(!output_to_reauth.contains("UnityGame"));
}

#[tokio::test]
async fn test_execute_chat_stream_graceful_budget_synthesis_and_activity() {
    let env = TestEnv::new("stream_graceful_synthesis");
    let data_root = env.data_root();
    let cfg = env.config_dir();

    let root = env.new_dir("BudgetSimRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    let proj = root.join("AdventureGame");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join("project.godot"),
        b"config_version=5\n[application]\nconfig/name=\"Adventure\"\n",
    )
    .unwrap();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "Discover game projects".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Synthesis Test".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let mut request_count = 0;
        while let Ok((mut socket, _)) = listener.accept().await {
            request_count += 1;
            let req_text = read_http_request(&mut socket).await;

            if request_count <= MAX_TOOL_ROUNDS {
                // Rounds 1..5: Model issues tool call
                let sse_obj = serde_json::json!({
                    "choices": [{
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "id": format!("call_{request_count}"),
                                "type": "function",
                                "function": {
                                    "name": "discover_game_projects",
                                    "arguments": "{}"
                                }
                            }]
                        },
                        "finish_reason": "tool_calls"
                    }]
                });
                let tool_call = format!("data: {}\n\n", serde_json::to_string(&sse_obj).unwrap());

                let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n";
                let _ = socket.write_all(header.as_bytes()).await;

                let chunk_hdr = format!("{:X}\r\n{}\r\n", tool_call.len(), tool_call);
                let _ = socket.write_all(chunk_hdr.as_bytes()).await;
                let _ = socket.write_all(b"0\r\n\r\n").await;
            } else if request_count == MAX_TOOL_ROUNDS + 1 {
                // Round 6: System dispatches final synthesis round!
                // Verify tools was NOT sent in the request body for synthesis round
                assert!(
                    !req_text.contains("\"tools\":["),
                    "Synthesis round must have tools disabled"
                );

                // Model outputs final synthesis text
                let ans = "data: {\"choices\":[{\"delta\":{\"content\":\"Found Godot project: Adventure.\"},\"finish_reason\":null}]}\n\n";
                let stop = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
                let done = "data: [DONE]\n\n";

                let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n";
                let _ = socket.write_all(header.as_bytes()).await;

                for p in [ans, stop, done] {
                    let chunk_hdr = format!("{:X}\r\n{}\r\n", p.len(), p);
                    let _ = socket.write_all(chunk_hdr.as_bytes()).await;
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                let _ = socket.write_all(b"0\r\n\r\n").await;
                break;
            }
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-synthesis-stream";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    super::commands::execute_chat_stream(
        sink.clone(),
        state.clone(),
        data_root.clone(),
        cfg.clone(),
        conv_id.clone(),
        req_id.to_string(),
        base_url,
        "test-model".to_string(),
        None,
        initial_conv.messages.clone(),
        cancel_rx,
    )
    .await;

    let events = sink.get_events();

    // 1. Verify toolActivity events were emitted
    let activity_events: Vec<&ToolActivityPayload> = events
        .iter()
        .filter_map(|e| e.tool_activity.as_ref())
        .collect();
    assert!(!activity_events.is_empty(), "Must emit toolActivity events");

    // Check first tool call emitted running then succeeded
    let call_1_events: Vec<&&ToolActivityPayload> = activity_events
        .iter()
        .filter(|a| a.id == "call_1")
        .collect();
    assert_eq!(call_1_events.len(), 2);
    assert_eq!(call_1_events[0].status, ToolActivityStatus::Running);
    assert_eq!(
        call_1_events[0].summary.as_deref(),
        Some("Discovering game projects in approved folders")
    );
    assert_eq!(call_1_events[1].status, ToolActivityStatus::Succeeded);
    assert_eq!(
        call_1_events[1].summary.as_deref(),
        Some("Found 1 game project(s)")
    );

    // 2. Verify synthesis text was emitted and stream completed
    let deltas: Vec<String> = events.iter().filter_map(|e| e.delta.clone()).collect();
    assert_eq!(deltas, vec!["Found Godot project: Adventure.".to_string()]);

    let terminal_events: Vec<&super::models::ChatStreamPayload> =
        events.iter().filter(|e| e.done == Some(true)).collect();
    assert_eq!(terminal_events.len(), 1);
    assert_eq!(terminal_events[0].status, Some(MessageStatus::Complete));
    assert_eq!(terminal_events[0].error, None);

    // 3. Verify history on disk contains only user message and synthesized assistant message
    let loaded = load_conversation_from_disk(&data_root, &conv_id).unwrap();
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[1].role, MessageRole::Assistant);
    assert_eq!(
        loaded.messages[1].content,
        "Found Godot project: Adventure."
    );
    assert_eq!(loaded.messages[1].status, MessageStatus::Complete);
}

#[test]
fn test_execute_tool_search_project_files_and_text() {
    let env = TestEnv::new("tool_exec_search");
    let cfg = env.config_dir();
    let root = env.new_dir("ToolSearchRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    let file = root.join("PlayerController.cs");
    std::fs::write(
        &file,
        b"public class PlayerController {\n    int health = 100;\n}\n",
    )
    .unwrap();

    // 1. search_project_files
    let files_args = serde_json::json!({
        "pattern": "*.cs",
        "match_type": "glob",
        "project_root": canon_root
    })
    .to_string();
    let files_out = execute_tool(&cfg, "search_project_files", &files_args);
    assert!(files_out.contains("PlayerController.cs"));
    assert!(files_out.contains("\"coverage\":\"complete\""));

    // 2. search_project_text
    let text_args = serde_json::json!({
        "query": "health",
        "project_root": canon_root
    })
    .to_string();
    let text_out = execute_tool(&cfg, "search_project_text", &text_args);
    assert!(text_out.contains("PlayerController.cs"));
    assert!(text_out.contains("health = 100"));
    assert!(text_out.contains("\"lineNumber\":2"));
}

#[test]
fn test_reauthorize_tool_result_search() {
    let env = TestEnv::new("tool_reauth_search");
    let cfg = env.config_dir();
    let root = env.new_dir("SearchReauth");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    let file = root.join("app.rs");
    std::fs::write(&file, b"fn confidential() {}\n").unwrap();

    let mut files_out = execute_tool(&cfg, "search_project_files", "{\"pattern\":\"app.rs\"}");
    assert!(files_out.contains("app.rs"));

    let mut text_out = execute_tool(&cfg, "search_project_text", "{\"query\":\"confidential\"}");
    assert!(text_out.contains("confidential"));

    // Revoke sharing
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(false)).unwrap();

    // Reauthorize before provider send
    reauthorize_tool_result(&cfg, "search_project_files", "{}", &mut files_out);
    assert!(files_out.contains("error"));
    assert!(
        !files_out.contains("app.rs"),
        "Must not leak filename when revoked"
    );

    reauthorize_tool_result(&cfg, "search_project_text", "{}", &mut text_out);
    assert!(text_out.contains("error"));
    assert!(
        !text_out.contains("confidential"),
        "Must not leak text snippet when revoked"
    );
}

#[tokio::test]
async fn test_execute_chat_stream_search_tool_flow() {
    let env = TestEnv::new("stream_search_flow");
    let data_root = env.data_root();
    let cfg = env.config_dir();

    let proj = env.new_dir("SearchApp");
    let code_file = proj.join("Player.cs");
    std::fs::write(&code_file, b"public int maxHealth = 100;\n").unwrap();
    let canon_proj = dunce::canonicalize(&proj)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &proj).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_proj, Some(true)).unwrap();

    let conv_id = uuid::Uuid::new_v4().to_string();
    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: "Find where maxHealth is defined".to_string(),
        status: MessageStatus::Complete,
    };
    let initial_conv = ConversationDetail {
        id: conv_id.clone(),
        title: "Search Test".to_string(),
        updated_at: "2026-10-19T10:00:00Z".to_string(),
        messages: vec![user_msg],
        ..Default::default()
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let mut request_count = 0;
        while let Ok((mut socket, _)) = listener.accept().await {
            request_count += 1;
            let req_text = read_http_request(&mut socket).await;

            if request_count == 1 {
                // Round 1: Model invokes search_project_text
                let sse_obj = serde_json::json!({
                    "choices": [{
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "id": "call_search_1",
                                "type": "function",
                                "function": {
                                    "name": "search_project_text",
                                    "arguments": "{\"query\":\"maxHealth\"}"
                                }
                            }]
                        },
                        "finish_reason": "tool_calls"
                    }]
                });
                let tool_call = format!("data: {}\n\n", serde_json::to_string(&sse_obj).unwrap());

                let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n";
                let _ = socket.write_all(header.as_bytes()).await;

                let chunk_hdr = format!("{:X}\r\n{}\r\n", tool_call.len(), tool_call);
                let _ = socket.write_all(chunk_hdr.as_bytes()).await;
                let _ = socket.write_all(b"0\r\n\r\n").await;
            } else if request_count == 2 {
                // Round 2: Verify tool result was sent back in tool role
                assert!(req_text.contains("\"role\":\"tool\""));
                assert!(req_text.contains("maxHealth"));
                assert!(req_text.contains("Player.cs"));

                // Model outputs response
                let ans = "data: {\"choices\":[{\"delta\":{\"content\":\"maxHealth is defined in Player.cs on line 1.\"},\"finish_reason\":null}]}\n\n";
                let stop = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
                let done = "data: [DONE]\n\n";

                let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n";
                let _ = socket.write_all(header.as_bytes()).await;

                for p in [ans, stop, done] {
                    let chunk_hdr = format!("{:X}\r\n{}\r\n", p.len(), p);
                    let _ = socket.write_all(chunk_hdr.as_bytes()).await;
                }
                let _ = socket.write_all(b"0\r\n\r\n").await;
                break;
            }
        }
    });

    let store = MockSecretStore::new();
    let state = ChatState::new(store);
    let req_id = "req-search-stream";

    let cancel_rx = state
        .try_start_request(req_id, &conv_id)
        .expect("Should acquire active lock");

    let sink = TestEventSink::new();
    let base_url = format!("http://{addr}/v1");

    super::commands::execute_chat_stream(
        sink.clone(),
        state.clone(),
        data_root.clone(),
        cfg.clone(),
        conv_id.clone(),
        req_id.to_string(),
        base_url,
        "test-model".to_string(),
        None,
        initial_conv.messages.clone(),
        cancel_rx,
    )
    .await;

    let events = sink.get_events();

    // Verify tool activity events
    let activity_events: Vec<&ToolActivityPayload> = events
        .iter()
        .filter_map(|e| e.tool_activity.as_ref())
        .collect();
    assert!(!activity_events.is_empty());
    assert_eq!(activity_events[0].status, ToolActivityStatus::Running);
    assert_eq!(
        activity_events[0].summary.as_deref(),
        Some("Searching project text")
    );
    assert_eq!(activity_events[1].status, ToolActivityStatus::Succeeded);
    assert!(
        activity_events[1]
            .summary
            .as_deref()
            .unwrap()
            .contains("Found 1 text match(es)")
    );

    // Verify final assistant message
    let deltas: Vec<String> = events.iter().filter_map(|e| e.delta.clone()).collect();
    assert_eq!(
        deltas,
        vec!["maxHealth is defined in Player.cs on line 1.".to_string()]
    );

    // Verify history on disk contains only user message and assistant message (no raw tool chunks)
    let loaded = load_conversation_from_disk(&data_root, &conv_id).unwrap();
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[1].role, MessageRole::Assistant);
    assert_eq!(
        loaded.messages[1].content,
        "maxHealth is defined in Player.cs on line 1."
    );
    assert_eq!(loaded.messages[1].status, MessageStatus::Complete);
}
