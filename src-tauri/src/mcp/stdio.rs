use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex as TokioMutex, mpsc};

use super::grants::{ensure_executable_is_approved, validate_and_canonicalize_executable};
use super::models::{
    DEFAULT_MCP_PROTOCOL_VERSION, MAX_ACTIVE_MCP_SESSIONS, MAX_MCP_RESPONSE_BYTES,
    MCP_REQUEST_TIMEOUT_MS, McpEditorBindingStatus, McpResourceMetadata, McpStdioCapabilities,
    McpStdioConnectionResult, McpToolMetadata, SUPPORTED_MCP_PROTOCOL_VERSIONS,
};
use super::protocol::{
    build_initialize_request, build_initialized_notification, is_supported_mcp_version,
};
use super::session::McpSessionManager;

/// Message sent from the stdout reader thread to the async session.
#[derive(Debug)]
pub enum StdoutMessage {
    Line(String),
    Oversized,
    Eof,
    IoError(String),
}

/// Represents an active native MCP stdio session.
/// Manages child process lifecycle, bounded newline JSON-RPC parsing, and safely drained stderr.
#[derive(Clone)]
pub struct McpStdioSession {
    pub session_id: String,
    pub canonical_path: String,
    pub protocol_version: String,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub child_pid: u32,
    child: Arc<std::sync::Mutex<Option<Child>>>,
    stdin: Arc<std::sync::Mutex<ChildStdin>>,
    stdout_rx: Arc<TokioMutex<mpsc::Receiver<StdoutMessage>>>,
    next_req_id: Arc<AtomicU64>,
    is_expired: Arc<AtomicBool>,
}

impl McpStdioSession {
    /// Kills and reaps the owned child process directly by PID, avoiding zombies.
    /// Never kills processes by name.
    pub fn kill_and_reap(&self) {
        self.is_expired.store(true, Ordering::SeqCst);
        if let Ok(mut lock) = self.child.lock() {
            if let Some(mut child) = lock.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    pub fn is_expired(&self) -> bool {
        self.is_expired.load(Ordering::SeqCst)
    }

    pub fn next_request_id(&self) -> u64 {
        self.next_req_id.fetch_add(1, Ordering::SeqCst)
    }

    pub fn send_line(&self, line: &str) -> Result<(), String> {
        if self.is_expired() {
            return Err("MCP stdio session is expired".to_string());
        }
        let mut stdin = self
            .stdin
            .lock()
            .map_err(|e| format!("Failed to lock stdin: {e}"))?;
        stdin
            .write_all(line.as_bytes())
            .map_err(|e| format!("Failed to write to stdio stdin: {e}"))?;
        stdin
            .flush()
            .map_err(|e| format!("Failed to flush stdio stdin: {e}"))?;
        Ok(())
    }

    pub async fn read_response_line(&self, timeout_ms: u64) -> Result<String, String> {
        let mut rx = self.stdout_rx.lock().await;
        match tokio::time::timeout(Duration::from_millis(timeout_ms), rx.recv()).await {
            Ok(Some(StdoutMessage::Line(line))) => Ok(line),
            Ok(Some(StdoutMessage::Oversized)) => Err(format!(
                "MCP response size exceeded limit of {MAX_MCP_RESPONSE_BYTES} bytes"
            )),
            Ok(Some(StdoutMessage::Eof)) => {
                Err("MCP stdio server process exited or closed stdout".to_string())
            }
            Ok(Some(StdoutMessage::IoError(e))) => {
                Err(format!("IO error reading stdio stream: {e}"))
            }
            Ok(None) => Err("MCP stdio channel closed".to_string()),
            Err(_) => Err(format!("MCP stdio request timed out after {timeout_ms}ms")),
        }
    }

    pub async fn send_jsonrpc_request(
        &self,
        request: &serde_json::Value,
        expected_id: u64,
    ) -> Result<serde_json::Value, String> {
        let serialized = serde_json::to_string(request)
            .map_err(|e| format!("Failed to serialize JSON-RPC request: {e}"))?;
        let line = format!("{serialized}\n");
        self.send_line(&line)?;

        let resp_line = self.read_response_line(MCP_REQUEST_TIMEOUT_MS).await?;
        let val: serde_json::Value = serde_json::from_str(&resp_line)
            .map_err(|e| format!("Invalid JSON in MCP stdio response: {e}"))?;

        if val.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
            return Err(
                "Invalid JSON-RPC protocol: missing or incorrect 'jsonrpc': '2.0' field in response"
                    .to_string(),
            );
        }

        if let Some(err_obj) = val.get("error") {
            let code = err_obj.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
            let message = err_obj
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown JSON-RPC error");
            return Err(format!("MCP error {code}: {message}"));
        }

        let resp_id = val.get("id").and_then(|v| v.as_u64());
        if resp_id != Some(expected_id) {
            return Err(format!(
                "Mismatched JSON-RPC response ID: expected {expected_id}, got {resp_id:?}"
            ));
        }

        Ok(val)
    }

    pub async fn send_jsonrpc_notification(
        &self,
        notification: &serde_json::Value,
    ) -> Result<(), String> {
        let serialized = serde_json::to_string(notification)
            .map_err(|e| format!("Failed to serialize JSON-RPC notification: {e}"))?;
        let line = format!("{serialized}\n");
        self.send_line(&line)?;
        Ok(())
    }
}

/// Spawns the approved executable child process with fixed argument `--mcp`.
/// Directly executes binary without shell and drains stderr in background without logging secrets.
pub fn spawn_stdio_process(
    canonical_path: &Path,
) -> Result<(Child, ChildStdin, mpsc::Receiver<StdoutMessage>), String> {
    let mut cmd = Command::new(canonical_path);
    // Fixed argument: strictly ["--mcp"]
    cmd.arg("--mcp");
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn MCP stdio executable: {e}"))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Failed to capture stdin of child process".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture stdout of child process".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Failed to capture stderr of child process".to_string())?;

    // Drain stderr safely in background thread without logging secrets or content
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut buf = [0u8; 1024];
        let mut drained = 0usize;
        const MAX_STDERR_DRAIN: usize = 16 * 1024;
        let mut safe_buffer = Vec::new();
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            if drained < MAX_STDERR_DRAIN {
                let take = (MAX_STDERR_DRAIN - drained).min(n);
                safe_buffer.extend_from_slice(&buf[..take]);
                drained += n;
            }
        }
    });

    // Read stdout lines bounded in background thread
    let (tx, rx) = mpsc::channel(32);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line_bytes = Vec::new();
            let mut byte_buf = [0u8; 1];
            let mut oversized = false;

            loop {
                match reader.read(&mut byte_buf) {
                    Ok(0) => {
                        if line_bytes.is_empty() {
                            let _ = tx.blocking_send(StdoutMessage::Eof);
                            return;
                        } else {
                            break;
                        }
                    }
                    Ok(_) => {
                        let b = byte_buf[0];
                        if b == b'\n' {
                            break;
                        }
                        if line_bytes.len() < MAX_MCP_RESPONSE_BYTES {
                            line_bytes.push(b);
                        } else {
                            oversized = true;
                        }
                    }
                    Err(e) => {
                        let _ = tx.blocking_send(StdoutMessage::IoError(e.to_string()));
                        return;
                    }
                }
            }

            if oversized {
                let _ = tx.blocking_send(StdoutMessage::Oversized);
            } else {
                match String::from_utf8(line_bytes) {
                    Ok(s) => {
                        let trimmed = s.trim_end_matches('\r').to_string();
                        if !trimmed.is_empty() {
                            if tx.blocking_send(StdoutMessage::Line(trimmed)).is_err() {
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.blocking_send(StdoutMessage::IoError(format!(
                            "Invalid UTF-8 in stdout line: {e}"
                        )));
                        return;
                    }
                }
            }
        }
    });

    Ok((child, stdin, rx))
}

/// Launches an approved local MCP stdio executable, performs initialize handshake,
/// verifies protocol version, sends initialized notification, and registers the session.
pub async fn connect_approved_mcp_stdio_core(
    config_dir: &Path,
    session_manager: &McpSessionManager,
    executable_path: &str,
) -> Result<McpStdioConnectionResult, String> {
    let canonical = validate_and_canonicalize_executable(executable_path)?;
    let canonical_str = canonical.to_string_lossy().to_string();
    ensure_executable_is_approved(config_dir, &canonical_str)?;

    if session_manager.total_active_count().await >= MAX_ACTIVE_MCP_SESSIONS {
        return Err(format!(
            "Maximum active MCP sessions limit ({MAX_ACTIVE_MCP_SESSIONS}) reached. Please disconnect an existing session first."
        ));
    }

    let (child, stdin, rx) = spawn_stdio_process(&canonical)?;
    let pid = child.id();

    let session_uuid = uuid::Uuid::new_v4().to_string();
    let session = McpStdioSession {
        session_id: session_uuid.clone(),
        canonical_path: canonical_str.clone(),
        protocol_version: DEFAULT_MCP_PROTOCOL_VERSION.to_string(),
        server_name: None,
        server_version: None,
        child_pid: pid,
        child: Arc::new(std::sync::Mutex::new(Some(child))),
        stdin: Arc::new(std::sync::Mutex::new(stdin)),
        stdout_rx: Arc::new(TokioMutex::new(rx)),
        next_req_id: Arc::new(AtomicU64::new(2)),
        is_expired: Arc::new(AtomicBool::new(false)),
    };

    // Send initialize request (id = 1)
    let init_req = build_initialize_request(1, DEFAULT_MCP_PROTOCOL_VERSION);
    let init_resp = match session.send_jsonrpc_request(&init_req, 1).await {
        Ok(resp) => resp,
        Err(e) => {
            session.kill_and_reap();
            return Err(e);
        }
    };

    let result = match init_resp.get("result") {
        Some(r) => r,
        None => {
            session.kill_and_reap();
            return Err("MCP initialize response missing 'result' object".to_string());
        }
    };

    let server_proto_ver = result
        .get("protocolVersion")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_MCP_PROTOCOL_VERSION);

    if !is_supported_mcp_version(server_proto_ver) {
        session.kill_and_reap();
        return Err(format!(
            "Unsupported MCP protocol version '{server_proto_ver}'. Supported versions: {}",
            SUPPORTED_MCP_PROTOCOL_VERSIONS.join(", ")
        ));
    }

    let server_info = result.get("serverInfo");
    let server_name = server_info
        .and_then(|si| si.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let server_version = server_info
        .and_then(|si| si.get("version"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut configured_session = session.clone();
    configured_session.protocol_version = server_proto_ver.to_string();
    configured_session.server_name = server_name.clone();
    configured_session.server_version = server_version.clone();

    // Send notifications/initialized
    let init_notif = build_initialized_notification();
    if let Err(e) = configured_session
        .send_jsonrpc_notification(&init_notif)
        .await
    {
        configured_session.kill_and_reap();
        return Err(format!("Failed to send initialized notification: {e}"));
    }

    session_manager
        .add_stdio_session(configured_session)
        .await?;

    Ok(McpStdioConnectionResult {
        session_id: session_uuid,
        canonical_path: canonical_str,
        transport: "stdio".to_string(),
        protocol_version: server_proto_ver.to_string(),
        server_name,
        server_version,
        process_id: pid,
    })
}

/// Queries tools and resources from an active official stdio session,
/// bounding metadata safely for future allowlisting and marking editor binding status honestly.
pub async fn list_stdio_capabilities_core(
    config_dir: &Path,
    session_manager: &McpSessionManager,
    session_id: &str,
) -> Result<McpStdioCapabilities, String> {
    let session = session_manager.get_stdio_session(session_id).await?;

    // Verify approval grant remains valid
    if let Err(e) = ensure_executable_is_approved(config_dir, &session.canonical_path) {
        session_manager.invalidate_stdio_session(session_id).await;
        return Err(e);
    }

    // 1. Query tools/list
    let tools_req_id = session.next_request_id();
    let tools_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": tools_req_id,
        "method": "tools/list"
    });

    let tools_resp = match session.send_jsonrpc_request(&tools_req, tools_req_id).await {
        Ok(resp) => resp,
        Err(e) => {
            if e.contains("exited") || e.contains("closed") {
                session_manager.invalidate_stdio_session(session_id).await;
            }
            return Err(e);
        }
    };

    let mut tools = Vec::new();
    const MAX_TOOLS_COUNT: usize = 100;
    const MAX_DESCRIPTION_LEN: usize = 500;

    if let Some(tool_arr) = tools_resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
    {
        for t in tool_arr.iter().take(MAX_TOOLS_COUNT) {
            if let Some(name) = t.get("name").and_then(|v| v.as_str()) {
                let description = t.get("description").and_then(|v| v.as_str()).map(|d| {
                    if d.chars().count() > MAX_DESCRIPTION_LEN {
                        format!(
                            "{}...",
                            d.chars().take(MAX_DESCRIPTION_LEN).collect::<String>()
                        )
                    } else {
                        d.to_string()
                    }
                });
                let input_schema = t.get("inputSchema").cloned();
                tools.push(McpToolMetadata {
                    name: name.to_string(),
                    description,
                    input_schema,
                });
            }
        }
    }

    // 2. Query resources/list
    let res_req_id = session.next_request_id();
    let res_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": res_req_id,
        "method": "resources/list"
    });

    let res_resp = match session.send_jsonrpc_request(&res_req, res_req_id).await {
        Ok(resp) => resp,
        Err(e) => {
            if e.contains("exited") || e.contains("closed") {
                session_manager.invalidate_stdio_session(session_id).await;
            }
            return Err(e);
        }
    };

    let mut resources = Vec::new();
    const MAX_RESOURCES_COUNT: usize = 100;

    if let Some(res_arr) = res_resp
        .get("result")
        .and_then(|r| r.get("resources"))
        .and_then(|t| t.as_array())
    {
        for r in res_arr.iter().take(MAX_RESOURCES_COUNT) {
            if let (Some(uri), Some(name)) = (
                r.get("uri").and_then(|v| v.as_str()),
                r.get("name").and_then(|v| v.as_str()),
            ) {
                let description = r.get("description").and_then(|v| v.as_str()).map(|d| {
                    if d.chars().count() > MAX_DESCRIPTION_LEN {
                        format!(
                            "{}...",
                            d.chars().take(MAX_DESCRIPTION_LEN).collect::<String>()
                        )
                    } else {
                        d.to_string()
                    }
                });
                let mime_type = r
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                resources.push(McpResourceMetadata {
                    uri: uri.to_string(),
                    name: name.to_string(),
                    description,
                    mime_type,
                });
            }
        }
    }

    Ok(McpStdioCapabilities {
        session_id: session_id.to_string(),
        canonical_path: session.canonical_path.clone(),
        protocol_version: session.protocol_version.clone(),
        tools,
        resources,
        editor_binding_status: McpEditorBindingStatus::PendingOrUndocumented {
            details: "Official Unity editor binding protocol is undocumented or pending editor-side approval in Unity Project Settings. Stopped at connection initialized and capability inventory with no false verified identity.".to_string(),
        },
        warnings: Vec::new(),
    })
}
