use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;

use super::models::{BoundUnityState, MAX_ACTIVE_MCP_SESSIONS, McpDisconnectResult};
use super::stdio::McpStdioSession;

/// Represents an in-memory active native MCP session.
/// Never persisted to disk or logged.
#[derive(Debug)]
pub struct McpSession {
    pub session_id: String,
    pub canonical_endpoint: String,
    pub protocol_version: String,
    pub mcp_session_id: Option<String>,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub bound_instance_id: Option<String>,
    pub bound_state: Option<BoundUnityState>,
    pub is_expired: bool,
    next_req_id: AtomicU64,
}

impl McpSession {
    pub fn new(
        session_id: String,
        canonical_endpoint: String,
        protocol_version: String,
        mcp_session_id: Option<String>,
        server_name: Option<String>,
        server_version: Option<String>,
    ) -> Self {
        Self {
            session_id,
            canonical_endpoint,
            protocol_version,
            mcp_session_id,
            server_name,
            server_version,
            bound_instance_id: None,
            bound_state: None,
            is_expired: false,
            next_req_id: AtomicU64::new(2),
        }
    }

    pub fn next_request_id(&self) -> u64 {
        self.next_req_id.fetch_add(1, Ordering::SeqCst)
    }

    pub fn mark_expired(&mut self) {
        self.is_expired = true;
        self.bound_instance_id = None;
        self.bound_state = None;
    }
}

/// Thread-safe in-memory manager for active MCP sessions (both HTTP and Stdio).
/// Bounded to `MAX_ACTIVE_MCP_SESSIONS` instances total.
#[derive(Default)]
pub struct McpSessionManager {
    http_sessions: RwLock<HashMap<String, McpSession>>,
    stdio_sessions: RwLock<HashMap<String, McpStdioSession>>,
}

impl McpSessionManager {
    pub fn new() -> Self {
        Self {
            http_sessions: RwLock::new(HashMap::new()),
            stdio_sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Adds a new HTTP session, enforcing maximum active session limits and purging expired ones.
    pub async fn add_session(&self, session: McpSession) -> Result<String, String> {
        let mut map = self.http_sessions.write().await;
        // Purge expired sessions
        map.retain(|_, s| !s.is_expired);

        let stdio_count = {
            let stdio_map = self.stdio_sessions.read().await;
            stdio_map.values().filter(|s| !s.is_expired()).count()
        };

        if map.len() + stdio_count >= MAX_ACTIVE_MCP_SESSIONS {
            return Err(format!(
                "Maximum active MCP sessions limit ({MAX_ACTIVE_MCP_SESSIONS}) reached. Please disconnect an existing session first."
            ));
        }

        let id = session.session_id.clone();
        map.insert(id.clone(), session);
        Ok(id)
    }

    /// Adds a new Stdio session, enforcing maximum active session limits and purging expired ones.
    pub async fn add_stdio_session(&self, session: McpStdioSession) -> Result<String, String> {
        let mut map = self.stdio_sessions.write().await;
        map.retain(|_, s| !s.is_expired());

        let http_count = {
            let http_map = self.http_sessions.read().await;
            http_map.values().filter(|s| !s.is_expired).count()
        };

        if map.len() + http_count >= MAX_ACTIVE_MCP_SESSIONS {
            return Err(format!(
                "Maximum active MCP sessions limit ({MAX_ACTIVE_MCP_SESSIONS}) reached. Please disconnect an existing session first."
            ));
        }

        let id = session.session_id.clone();
        map.insert(id.clone(), session);
        Ok(id)
    }

    /// Removes an HTTP session by its session ID.
    pub async fn remove_session(&self, session_id: &str) -> bool {
        let mut map = self.http_sessions.write().await;
        map.remove(session_id).is_some()
    }

    /// Removes a Stdio session by its session ID, terminating and reaping the child process.
    pub async fn remove_stdio_session(&self, session_id: &str) -> bool {
        let mut map = self.stdio_sessions.write().await;
        if let Some(s) = map.remove(session_id) {
            s.kill_and_reap();
            true
        } else {
            false
        }
    }

    /// Disconnects any active session (Stdio or HTTP) by session ID.
    pub async fn disconnect_any(&self, session_id: &str) -> McpDisconnectResult {
        // Check stdio first
        {
            let mut stdio_map = self.stdio_sessions.write().await;
            if let Some(s) = stdio_map.remove(session_id) {
                s.kill_and_reap();
                return McpDisconnectResult {
                    session_id: session_id.to_string(),
                    success: true,
                    message: "MCP stdio session disconnected and process terminated cleanly."
                        .to_string(),
                };
            }
        }

        // Check http
        {
            let mut http_map = self.http_sessions.write().await;
            if let Some(mut s) = http_map.remove(session_id) {
                s.mark_expired();
                return McpDisconnectResult {
                    session_id: session_id.to_string(),
                    success: true,
                    message: "MCP session disconnected cleanly.".to_string(),
                };
            }
        }

        McpDisconnectResult {
            session_id: session_id.to_string(),
            success: false,
            message: "MCP session was not found or already disconnected.".to_string(),
        }
    }

    /// Invalidates and removes all sessions associated with a canonical endpoint URL.
    /// Called when an endpoint grant is revoked.
    pub async fn invalidate_endpoint_sessions(&self, canonical_endpoint: &str) {
        let mut map = self.http_sessions.write().await;
        for s in map.values_mut() {
            if s.canonical_endpoint == canonical_endpoint {
                s.mark_expired();
            }
        }
        map.retain(|_, s| s.canonical_endpoint != canonical_endpoint);
    }

    /// Invalidates and removes all stdio sessions associated with a canonical executable path.
    /// Called when an executable grant is revoked. Kills and reaps child processes.
    pub async fn invalidate_executable_sessions(&self, canonical_path: &str) {
        let mut map = self.stdio_sessions.write().await;
        for s in map.values() {
            if s.canonical_path == canonical_path {
                s.kill_and_reap();
            }
        }
        map.retain(|_, s| s.canonical_path != canonical_path);
    }

    /// Marks a specific HTTP session as expired and removes it.
    pub async fn invalidate_session(&self, session_id: &str) {
        let mut map = self.http_sessions.write().await;
        if let Some(s) = map.get_mut(session_id) {
            s.mark_expired();
        }
        map.remove(session_id);
    }

    /// Marks a specific stdio session as expired, kills child, and removes it.
    pub async fn invalidate_stdio_session(&self, session_id: &str) {
        let mut map = self.stdio_sessions.write().await;
        if let Some(s) = map.remove(session_id) {
            s.kill_and_reap();
        }
    }

    /// Retrieves an active stdio session clone for async operations.
    pub async fn get_stdio_session(&self, session_id: &str) -> Result<McpStdioSession, String> {
        let map = self.stdio_sessions.read().await;
        let s = map.get(session_id).ok_or_else(|| {
            format!("MCP stdio session '{session_id}' not found or has been disconnected.")
        })?;
        if s.is_expired() {
            return Err(format!("MCP stdio session '{session_id}' is expired."));
        }
        Ok(s.clone())
    }

    /// Runs a closure with a read reference to an HTTP session.
    pub async fn with_session<F, R>(&self, session_id: &str, f: F) -> Result<R, String>
    where
        F: FnOnce(&McpSession) -> Result<R, String>,
    {
        let map = self.http_sessions.read().await;
        let session = map.get(session_id).ok_or_else(|| {
            format!("MCP session '{session_id}' not found or has been disconnected.")
        })?;
        if session.is_expired {
            return Err(format!(
                "MCP session '{session_id}' is expired. A fresh connection and explicit rebind is required."
            ));
        }
        f(session)
    }

    /// Runs a closure with a mutable reference to an HTTP session.
    pub async fn with_session_mut<F, R>(&self, session_id: &str, f: F) -> Result<R, String>
    where
        F: FnOnce(&mut McpSession) -> Result<R, String>,
    {
        let mut map = self.http_sessions.write().await;
        let session = map.get_mut(session_id).ok_or_else(|| {
            format!("MCP session '{session_id}' not found or has been disconnected.")
        })?;
        if session.is_expired {
            return Err(format!(
                "MCP session '{session_id}' is expired. A fresh connection and explicit rebind is required."
            ));
        }
        f(session)
    }

    /// Returns the number of currently active HTTP sessions.
    pub async fn active_count(&self) -> usize {
        let map = self.http_sessions.read().await;
        map.values().filter(|s| !s.is_expired).count()
    }

    /// Returns the total number of currently active sessions across HTTP and Stdio.
    pub async fn total_active_count(&self) -> usize {
        let http_count = {
            let map = self.http_sessions.read().await;
            map.values().filter(|s| !s.is_expired).count()
        };
        let stdio_count = {
            let map = self.stdio_sessions.read().await;
            map.values().filter(|s| !s.is_expired()).count()
        };
        http_count + stdio_count
    }
}
