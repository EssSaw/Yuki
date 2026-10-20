use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;

use super::models::{BoundUnityState, MAX_ACTIVE_MCP_SESSIONS};

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

/// Thread-safe in-memory manager for active MCP sessions.
/// Bounded to `MAX_ACTIVE_MCP_SESSIONS` instances.
#[derive(Default)]
pub struct McpSessionManager {
    sessions: RwLock<HashMap<String, McpSession>>,
}

impl McpSessionManager {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Adds a new session, enforcing maximum active session limits and purging expired ones.
    pub async fn add_session(&self, session: McpSession) -> Result<String, String> {
        let mut map = self.sessions.write().await;
        // Purge expired sessions
        map.retain(|_, s| !s.is_expired);

        if map.len() >= MAX_ACTIVE_MCP_SESSIONS {
            return Err(format!(
                "Maximum active MCP sessions limit ({MAX_ACTIVE_MCP_SESSIONS}) reached. Please disconnect an existing session first."
            ));
        }

        let id = session.session_id.clone();
        map.insert(id.clone(), session);
        Ok(id)
    }

    /// Removes a session by its session ID.
    pub async fn remove_session(&self, session_id: &str) -> bool {
        let mut map = self.sessions.write().await;
        map.remove(session_id).is_some()
    }

    /// Invalidates and removes all sessions associated with a canonical endpoint URL.
    /// Called when an endpoint grant is revoked.
    pub async fn invalidate_endpoint_sessions(&self, canonical_endpoint: &str) {
        let mut map = self.sessions.write().await;
        for s in map.values_mut() {
            if s.canonical_endpoint == canonical_endpoint {
                s.mark_expired();
            }
        }
        map.retain(|_, s| s.canonical_endpoint != canonical_endpoint);
    }

    /// Marks a specific session as expired and removes it.
    pub async fn invalidate_session(&self, session_id: &str) {
        let mut map = self.sessions.write().await;
        if let Some(s) = map.get_mut(session_id) {
            s.mark_expired();
        }
        map.remove(session_id);
    }

    /// Runs a closure with a read reference to a session.
    pub async fn with_session<F, R>(&self, session_id: &str, f: F) -> Result<R, String>
    where
        F: FnOnce(&McpSession) -> Result<R, String>,
    {
        let map = self.sessions.read().await;
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

    /// Runs a closure with a mutable reference to a session.
    pub async fn with_session_mut<F, R>(&self, session_id: &str, f: F) -> Result<R, String>
    where
        F: FnOnce(&mut McpSession) -> Result<R, String>,
    {
        let mut map = self.sessions.write().await;
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

    /// Returns the number of currently active sessions.
    pub async fn active_count(&self) -> usize {
        let map = self.sessions.read().await;
        map.values().filter(|s| !s.is_expired).count()
    }
}
