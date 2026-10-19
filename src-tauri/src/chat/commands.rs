use futures_util::StreamExt;
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, State};

use super::client::{
    ChatCompletionRequest, MAX_HISTORY_CHARS, MAX_INPUT_CHARS, MAX_STREAM_BYTES, fetch_models,
    prepare_bounded_messages, validate_endpoint,
};
use super::credentials::{SERVICE_NAME, USER_NAME};
use super::history::{
    delete_conversation_from_disk, list_conversations_from_disk, load_conversation_from_disk,
    save_conversation_atomic, validate_conversation_id,
};
use super::models::{
    ChatConfig, ChatMessage, ChatSettingsDto, ChatStreamPayload, ConversationDetail,
    ConversationSummary, MessageRole, MessageStatus, StartChatResponse,
};
use super::parser::{ParsedEvent, SseParser};
use super::state::ChatState;

const SETTINGS_FILE_NAME: &str = "chat_settings.json";

/// Loads the chat configuration from the app config directory.
pub fn load_chat_config(config_dir: &Path) -> Result<ChatConfig, String> {
    let path = config_dir.join(SETTINGS_FILE_NAME);
    if !path.exists() {
        return Ok(ChatConfig::default());
    }
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("Failed to read chat settings: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse chat settings: {e}"))
}

/// Atomically saves the chat configuration to chat_settings.json with backup recovery.
pub fn save_chat_config_atomic(config_dir: &Path, config: &ChatConfig) -> Result<(), String> {
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("Failed to create config directory: {e}"))?;

    let dest = config_dir.join(SETTINGS_FILE_NAME);
    let json_bytes = serde_json::to_vec_pretty(config)
        .map_err(|e| format!("Failed to serialize chat settings: {e}"))?;

    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let temp_path = config_dir.join(format!(".{SETTINGS_FILE_NAME}.tmp_{pid}_{nanos}"));
    let backup_path = config_dir.join(format!(".{SETTINGS_FILE_NAME}.bak_{pid}_{nanos}"));

    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|e| format!("Failed to create temporary settings file: {e}"))?;
        file.write_all(&json_bytes)
            .map_err(|e| format!("Failed to write settings: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("Failed to sync settings file: {e}"))?;
    }

    if dest.exists() {
        std::fs::rename(&dest, &backup_path).map_err(|e| {
            let _ = std::fs::remove_file(&temp_path);
            format!("Failed to move existing settings to backup: {e}")
        })?;

        if let Err(e) = std::fs::rename(&temp_path, &dest) {
            let _ = std::fs::rename(&backup_path, &dest);
            let _ = std::fs::remove_file(&temp_path);
            return Err(format!("Failed to replace settings file: {e}"));
        }

        let _ = std::fs::remove_file(&backup_path);
    } else if let Err(e) = std::fs::rename(&temp_path, &dest) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Failed to save settings: {e}"));
    }

    Ok(())
}

fn generate_conversation_title(content: &str) -> String {
    let first_line = content.lines().next().unwrap_or("New Conversation").trim();
    if first_line.is_empty() {
        return "New Conversation".to_string();
    }
    let chars: Vec<char> = first_line.chars().collect();
    if chars.len() > 40 {
        let prefix: String = chars[..40].iter().collect();
        format!("{prefix}...")
    } else {
        first_line.to_string()
    }
}

fn sanitize_reqwest_error(e: reqwest::Error, base_url: &str) -> String {
    if e.is_connect() {
        format!(
            "Failed to connect to 9router at '{base_url}'. Ensure the service is running and reachable."
        )
    } else if e.is_timeout() {
        "Request to 9router timed out.".to_string()
    } else {
        "Network communication failure with 9router.".to_string()
    }
}

fn sanitize_http_status(status: reqwest::StatusCode, model: &str) -> String {
    match status {
        reqwest::StatusCode::UNAUTHORIZED => {
            "Authentication failed (HTTP 401). Please check your API key in settings.".to_string()
        }
        reqwest::StatusCode::FORBIDDEN => "Access forbidden (HTTP 403) by provider.".to_string(),
        reqwest::StatusCode::NOT_FOUND => {
            format!("Model '{model}' or endpoint was not found (HTTP 404).")
        }
        reqwest::StatusCode::TOO_MANY_REQUESTS => {
            "Rate limit exceeded (HTTP 429). Please try again shortly.".to_string()
        }
        _ => format!("Provider returned HTTP status {status}."),
    }
}

#[tauri::command]
pub async fn get_chat_settings(
    app: AppHandle,
    state: State<'_, ChatState>,
) -> Result<ChatSettingsDto, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let config = load_chat_config(&config_dir)?;
    let has_api_key = state
        .secret_store()
        .get_api_key(SERVICE_NAME, USER_NAME)?
        .is_some();

    Ok(ChatSettingsDto {
        base_url: config.base_url,
        model: config.model,
        has_api_key,
    })
}

#[tauri::command]
pub async fn save_chat_settings(
    app: AppHandle,
    state: State<'_, ChatState>,
    base_url: String,
    model: String,
    api_key: Option<String>,
    clear_api_key: Option<bool>,
) -> Result<ChatSettingsDto, String> {
    validate_endpoint(&base_url)?;

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;

    if clear_api_key == Some(true) {
        state
            .secret_store()
            .delete_api_key(SERVICE_NAME, USER_NAME)?;
    } else if let Some(ref key) = api_key {
        let trimmed_key = key.trim();
        if !trimmed_key.is_empty() {
            state
                .secret_store()
                .set_api_key(SERVICE_NAME, USER_NAME, trimmed_key)?;
        }
    }

    let config = ChatConfig {
        base_url: base_url.trim().to_string(),
        model: model.trim().to_string(),
    };
    save_chat_config_atomic(&config_dir, &config)?;

    let has_api_key = state
        .secret_store()
        .get_api_key(SERVICE_NAME, USER_NAME)?
        .is_some();

    Ok(ChatSettingsDto {
        base_url: config.base_url,
        model: config.model,
        has_api_key,
    })
}

#[tauri::command]
pub async fn list_chat_models(
    app: AppHandle,
    state: State<'_, ChatState>,
) -> Result<Vec<String>, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let config = load_chat_config(&config_dir)?;
    let api_key = state.secret_store().get_api_key(SERVICE_NAME, USER_NAME)?;

    fetch_models(state.http_client(), &config.base_url, api_key.as_deref()).await
}

#[tauri::command]
pub async fn list_conversations(app: AppHandle) -> Result<Vec<ConversationSummary>, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let workspace_settings = crate::workspace::settings::load_settings(&config_dir)?;

    let data_root_str = workspace_settings.data_root.ok_or(
        "Data root is not configured or unavailable. Please choose a data root in workspace settings first."
            .to_string(),
    )?;

    let data_root = PathBuf::from(data_root_str);
    tauri::async_runtime::spawn_blocking(move || list_conversations_from_disk(&data_root))
        .await
        .map_err(|e| format!("Failed to read conversations: {e}"))?
}

#[tauri::command]
pub async fn load_conversation(app: AppHandle, id: String) -> Result<ConversationDetail, String> {
    validate_conversation_id(&id)?;

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let workspace_settings = crate::workspace::settings::load_settings(&config_dir)?;

    let data_root_str = workspace_settings.data_root.ok_or(
        "Data root is not configured or unavailable. Please choose a data root in workspace settings first."
            .to_string(),
    )?;

    let data_root = PathBuf::from(data_root_str);
    tauri::async_runtime::spawn_blocking(move || load_conversation_from_disk(&data_root, &id))
        .await
        .map_err(|e| format!("Failed to load conversation: {e}"))?
}

#[tauri::command]
pub async fn delete_conversation(
    app: AppHandle,
    state: State<'_, ChatState>,
    id: String,
) -> Result<(), String> {
    let valid_uuid = validate_conversation_id(&id)?;
    let valid_uuid_str = valid_uuid.to_string();

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let workspace_settings = crate::workspace::settings::load_settings(&config_dir)?;

    let data_root_str = workspace_settings.data_root.ok_or(
        "Data root is not configured or unavailable. Please choose a data root in workspace settings first."
            .to_string(),
    )?;

    let data_root = PathBuf::from(data_root_str);
    if !data_root.exists() {
        return Err(format!(
            "Configured data root directory '{}' is unavailable.",
            data_root.display()
        ));
    }

    let chat_state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let active_guard = chat_state.inner.active.lock().map_err(|e| e.to_string())?;
        if let Some(ref active) = *active_guard {
            return Err(format!(
                "Cannot delete conversation while a chat request ('{}') is active. Please wait or cancel it before deleting.",
                active.request_id
            ));
        }

        delete_conversation_from_disk(&data_root, &valid_uuid_str)
    })
    .await
    .map_err(|e| format!("Delete conversation task failed: {e}"))?
}

#[tauri::command]
pub async fn start_chat(
    app: AppHandle,
    state: State<'_, ChatState>,
    conversation_id: Option<String>,
    content: String,
    request_id: String,
) -> Result<StartChatResponse, String> {
    let trimmed_content = content.trim();
    if trimmed_content.is_empty() {
        return Err("Message content cannot be empty.".to_string());
    }
    if trimmed_content.len() > MAX_INPUT_CHARS {
        return Err(format!(
            "Message length exceeds maximum allowed character limit ({}).",
            MAX_INPUT_CHARS
        ));
    }

    let trimmed_req_id = request_id.trim();
    if trimmed_req_id.is_empty() {
        return Err("Request ID cannot be empty.".to_string());
    }

    // 1. Resolve conversation ID
    let (resolved_id, is_new) = match conversation_id {
        Some(ref id) => {
            validate_conversation_id(id)?;
            (id.clone(), false)
        }
        None => (uuid::Uuid::new_v4().to_string(), true),
    };

    // 2. Validate workspace data root
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let workspace_settings = crate::workspace::settings::load_settings(&config_dir)?;
    let data_root_str = workspace_settings.data_root.ok_or(
        "Data root is not configured or unavailable. Please choose a data root in workspace settings first."
            .to_string(),
    )?;
    let data_root = PathBuf::from(data_root_str);
    if !data_root.exists() {
        return Err(format!(
            "Configured data root directory '{}' is unavailable.",
            data_root.display()
        ));
    }

    // 3. Validate chat config
    let chat_config = load_chat_config(&config_dir)?;
    if chat_config.base_url.trim().is_empty() {
        return Err(
            "Chat endpoint URL is not configured. Please set an endpoint in settings.".to_string(),
        );
    }
    validate_endpoint(&chat_config.base_url)?;
    if chat_config.model.trim().is_empty() {
        return Err(
            "Chat model is not configured. Please select or enter a model in settings.".to_string(),
        );
    }

    let bg_api_key = state.secret_store().get_api_key(SERVICE_NAME, USER_NAME)?;

    // 4. Try register active request guard
    let cancel_rx = state.try_start_request(trimmed_req_id, &resolved_id)?;

    // 5. Pre-persist user message before any network transmission ("Save user before network")
    let user_msg_content = trimmed_content.to_string();
    let data_root_clone = data_root.clone();
    let cid_clone = resolved_id.clone();

    let conv_result = tauri::async_runtime::spawn_blocking(move || {
        let mut conv = if is_new {
            ConversationDetail {
                id: cid_clone.clone(),
                title: generate_conversation_title(&user_msg_content),
                updated_at: chrono::Utc::now().to_rfc3339(),
                messages: Vec::new(),
            }
        } else {
            load_conversation_from_disk(&data_root_clone, &cid_clone)?
        };

        let user_msg = ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            content: user_msg_content,
            status: MessageStatus::Complete,
        };
        conv.messages.push(user_msg);
        conv.updated_at = chrono::Utc::now().to_rfc3339();

        save_conversation_atomic(&data_root_clone, &conv)?;
        Ok::<ConversationDetail, String>(conv)
    })
    .await
    .map_err(|e| {
        state.finish_request(trimmed_req_id);
        format!("Persistence task error: {e}")
    })?;

    let conv = match conv_result {
        Ok(c) => c,
        Err(e) => {
            state.finish_request(trimmed_req_id);
            return Err(e);
        }
    };

    // 6. Spawn background streaming task and return promptly
    let sink = std::sync::Arc::new(TauriEventSink::new(app.clone()));
    let state_inner = state.inner().clone();
    let bg_data_root = data_root.clone();
    let bg_conv_id = resolved_id.clone();
    let bg_req_id = trimmed_req_id.to_string();
    let bg_base_url = chat_config.base_url;
    let bg_model = chat_config.model;
    let bg_messages = conv.messages;

    tauri::async_runtime::spawn(async move {
        execute_chat_stream(
            sink,
            state_inner,
            bg_data_root,
            bg_conv_id,
            bg_req_id,
            bg_base_url,
            bg_model,
            bg_api_key,
            bg_messages,
            cancel_rx,
        )
        .await;
    });

    Ok(StartChatResponse {
        conversation_id: resolved_id,
    })
}

#[tauri::command]
pub fn cancel_chat(
    _app: AppHandle,
    state: State<'_, ChatState>,
    request_id: String,
) -> Result<(), String> {
    state.cancel_request(&request_id);
    Ok(())
}

async fn save_terminal_assistant(
    data_root: &Path,
    conv_id: &str,
    assistant_id: String,
    content: String,
    status: MessageStatus,
) -> Result<(), String> {
    let data_root = data_root.to_path_buf();
    let conv_id = conv_id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conv = load_conversation_from_disk(&data_root, &conv_id)?;
        conv.messages.push(ChatMessage {
            id: assistant_id,
            role: MessageRole::Assistant,
            content,
            status,
        });
        conv.updated_at = chrono::Utc::now().to_rfc3339();
        save_conversation_atomic(&data_root, &conv)
    })
    .await
    .map_err(|_| "History write task failed.".to_string())?
}

pub trait ChatEventSink: Send + Sync {
    fn emit_event(&self, payload: ChatStreamPayload);
}

pub struct TauriEventSink {
    app: AppHandle,
}

impl TauriEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ChatEventSink for TauriEventSink {
    fn emit_event(&self, payload: ChatStreamPayload) {
        let _ = self.app.emit("chat-stream", payload);
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_chat_stream(
    sink: std::sync::Arc<dyn ChatEventSink>,
    state: ChatState,
    data_root: PathBuf,
    conv_id: String,
    req_id: String,
    base_url: String,
    model: String,
    api_key: Option<String>,
    history_messages: Vec<ChatMessage>,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
) {
    let assistant_id = uuid::Uuid::new_v4().to_string();
    let mut accumulated_content = String::new();

    // Check early cancellation
    if *cancel_rx.borrow() {
        let _ = save_terminal_assistant(
            &data_root,
            &conv_id,
            assistant_id,
            accumulated_content,
            MessageStatus::Cancelled,
        )
        .await;
        sink.emit_event(ChatStreamPayload {
            request_id: req_id.clone(),
            conversation_id: conv_id,
            delta: None,
            done: Some(true),
            error: Some("Chat was cancelled.".to_string()),
            status: Some(MessageStatus::Cancelled),
        });
        state.finish_request(&req_id);
        return;
    }

    let bounded = prepare_bounded_messages(&history_messages, MAX_HISTORY_CHARS);
    let base = base_url.trim_end_matches('/');
    let completions_url = format!("{base}/chat/completions");

    let req_body = ChatCompletionRequest {
        model: model.clone(),
        messages: bounded,
        stream: true,
    };

    let mut http_req = state.http_client().post(&completions_url).json(&req_body);
    if let Some(ref key) = api_key {
        let trimmed_key = key.trim();
        if !trimmed_key.is_empty() {
            http_req = http_req.header("Authorization", format!("Bearer {trimmed_key}"));
        }
    }

    let send_result = http_req.send().await;
    let resp = match send_result {
        Ok(r) => r,
        Err(e) => {
            let sanitized = sanitize_reqwest_error(e, base);
            let _ = save_terminal_assistant(
                &data_root,
                &conv_id,
                assistant_id,
                accumulated_content,
                MessageStatus::Failed,
            )
            .await;
            sink.emit_event(ChatStreamPayload {
                request_id: req_id.clone(),
                conversation_id: conv_id,
                delta: None,
                done: Some(true),
                error: Some(sanitized),
                status: Some(MessageStatus::Failed),
            });
            state.finish_request(&req_id);
            return;
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let sanitized = sanitize_http_status(status, &model);
        let _ = save_terminal_assistant(
            &data_root,
            &conv_id,
            assistant_id,
            accumulated_content,
            MessageStatus::Failed,
        )
        .await;
        sink.emit_event(ChatStreamPayload {
            request_id: req_id.clone(),
            conversation_id: conv_id,
            delta: None,
            done: Some(true),
            error: Some(sanitized),
            status: Some(MessageStatus::Failed),
        });
        state.finish_request(&req_id);
        return;
    }

    let mut stream = resp.bytes_stream();
    let mut parser = SseParser::new();
    let mut total_bytes = 0usize;
    let mut stream_completed = false;

    loop {
        if *cancel_rx.borrow() {
            let _ = save_terminal_assistant(
                &data_root,
                &conv_id,
                assistant_id,
                accumulated_content,
                MessageStatus::Cancelled,
            )
            .await;
            sink.emit_event(ChatStreamPayload {
                request_id: req_id.clone(),
                conversation_id: conv_id,
                delta: None,
                done: Some(true),
                error: Some("Chat was cancelled.".to_string()),
                status: Some(MessageStatus::Cancelled),
            });
            state.finish_request(&req_id);
            return;
        }

        tokio::select! {
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    let _ = save_terminal_assistant(
                        &data_root,
                        &conv_id,
                        assistant_id,
                        accumulated_content,
                        MessageStatus::Cancelled,
                    )
                    .await;
                    sink.emit_event(ChatStreamPayload {
                        request_id: req_id.clone(),
                        conversation_id: conv_id,
                        delta: None,
                        done: Some(true),
                        error: Some("Chat was cancelled.".to_string()),
                        status: Some(MessageStatus::Cancelled),
                    });
                    state.finish_request(&req_id);
                    return;
                }
            }
            item = stream.next() => {
                match item {
                    Some(Ok(bytes)) => {
                        total_bytes += bytes.len();
                        if total_bytes > MAX_STREAM_BYTES {
                            let _ = save_terminal_assistant(
                                &data_root,
                                &conv_id,
                                assistant_id,
                                accumulated_content,
                                MessageStatus::Failed,
                            )
                            .await;
                            sink.emit_event(ChatStreamPayload {
                                request_id: req_id.clone(),
                                conversation_id: conv_id,
                                delta: None,
                                done: Some(true),
                                error: Some("Response exceeded maximum allowed size (5MB).".to_string()),
                                status: Some(MessageStatus::Failed),
                            });
                            state.finish_request(&req_id);
                            return;
                        }

                        let text = String::from_utf8_lossy(&bytes);
                        let events = parser.feed(&text);
                        for event in events {
                            match event {
                                ParsedEvent::Delta(delta) => {
                                    accumulated_content.push_str(&delta);
                                    sink.emit_event(ChatStreamPayload {
                                        request_id: req_id.clone(),
                                        conversation_id: conv_id.clone(),
                                        delta: Some(delta),
                                        done: None,
                                        error: None,
                                        status: None,
                                    });
                                }
                                ParsedEvent::Done => {
                                    stream_completed = true;
                                }
                                ParsedEvent::Error(err) => {
                                    let _ = save_terminal_assistant(
                                        &data_root,
                                        &conv_id,
                                        assistant_id,
                                        accumulated_content,
                                        MessageStatus::Failed,
                                    )
                                    .await;
                                    sink.emit_event(ChatStreamPayload {
                                        request_id: req_id.clone(),
                                        conversation_id: conv_id,
                                        delta: None,
                                        done: Some(true),
                                        error: Some(err),
                                        status: Some(MessageStatus::Failed),
                                    });
                                    state.finish_request(&req_id);
                                    return;
                                }
                            }
                        }

                        if stream_completed {
                            break;
                        }
                    }
                    Some(Err(_)) => {
                        let err_msg = "Stream connection interrupted.".to_string();
                        let _ = save_terminal_assistant(
                            &data_root,
                            &conv_id,
                            assistant_id,
                            accumulated_content,
                            MessageStatus::Failed,
                        )
                        .await;
                        sink.emit_event(ChatStreamPayload {
                            request_id: req_id.clone(),
                            conversation_id: conv_id,
                            delta: None,
                            done: Some(true),
                            error: Some(err_msg),
                            status: Some(MessageStatus::Failed),
                        });
                        state.finish_request(&req_id);
                        return;
                    }
                    None => {
                        break;
                    }
                }
            }
        }
    }

    let tail_events = parser.finish();
    for event in tail_events {
        match event {
            ParsedEvent::Delta(delta) => {
                accumulated_content.push_str(&delta);
                sink.emit_event(ChatStreamPayload {
                    request_id: req_id.clone(),
                    conversation_id: conv_id.clone(),
                    delta: Some(delta),
                    done: None,
                    error: None,
                    status: None,
                });
            }
            ParsedEvent::Done => {
                stream_completed = true;
            }
            ParsedEvent::Error(err) => {
                let _ = save_terminal_assistant(
                    &data_root,
                    &conv_id,
                    assistant_id,
                    accumulated_content,
                    MessageStatus::Failed,
                )
                .await;
                sink.emit_event(ChatStreamPayload {
                    request_id: req_id.clone(),
                    conversation_id: conv_id,
                    delta: None,
                    done: Some(true),
                    error: Some(err),
                    status: Some(MessageStatus::Failed),
                });
                state.finish_request(&req_id);
                return;
            }
        }
    }

    if !stream_completed {
        let _ = save_terminal_assistant(
            &data_root,
            &conv_id,
            assistant_id,
            accumulated_content,
            MessageStatus::Failed,
        )
        .await;
        sink.emit_event(ChatStreamPayload {
            request_id: req_id.clone(),
            conversation_id: conv_id,
            delta: None,
            done: Some(true),
            error: Some("Stream ended before completion was confirmed.".into()),
            status: Some(MessageStatus::Failed),
        });
        state.finish_request(&req_id);
        return;
    }

    // Normal completion
    let saved = save_terminal_assistant(
        &data_root,
        &conv_id,
        assistant_id,
        accumulated_content,
        MessageStatus::Complete,
    )
    .await;
    sink.emit_event(ChatStreamPayload {
        request_id: req_id.clone(),
        conversation_id: conv_id,
        delta: None,
        done: Some(true),
        error: saved.as_ref().err().map(|_| "Response received but history could not be saved. Do not assume it will survive restart.".to_string()),
        status: Some(if saved.is_ok() { MessageStatus::Complete } else { MessageStatus::Failed }),
    });
    state.finish_request(&req_id);
}
