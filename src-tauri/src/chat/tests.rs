use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::client::{
    MAX_HISTORY_CHARS, MAX_INPUT_CHARS, build_http_client, fetch_models, prepare_bounded_messages,
    validate_endpoint,
};
use super::commands::{load_chat_config, save_chat_config_atomic};
use super::credentials::{MockSecretStore, SERVICE_NAME, SecretStore, USER_NAME};
use super::history::{
    get_conversations_dir, list_conversations_from_disk, load_conversation_from_disk,
    save_conversation_atomic, validate_conversation_id,
};
use super::models::{ChatConfig, ChatMessage, ConversationDetail, MessageRole, MessageStatus};
use super::parser::{ParsedEvent, SseParser};
use super::state::ChatState;

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
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    // Start mock streaming SSE server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;

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
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;

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
    let conv_id_clone = conv_id.clone();
    let initial_messages = initial_conv.messages.clone();

    let handle = tokio::spawn(async move {
        super::commands::execute_chat_stream(
            sink_clone,
            state_clone,
            data_root_clone,
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
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;

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
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    // Start mock streaming SSE server that sends finish_reason: "stop" without [DONE]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;

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
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    // Mock server sends partial delta, then closes abruptly without finish_reason or [DONE]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;

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
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;

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
    };
    save_conversation_atomic(&data_root, &initial_conv).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;

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
