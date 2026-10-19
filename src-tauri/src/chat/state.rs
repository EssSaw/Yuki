use std::sync::{Arc, Mutex};
use tokio::sync::watch;

use super::client::build_http_client;
use super::credentials::{KeyringSecretStore, SecretStore};

pub struct ActiveRequest {
    pub request_id: String,
    pub conversation_id: String,
    pub cancel_tx: watch::Sender<bool>,
}

pub struct ChatStateInner {
    pub active: Mutex<Option<ActiveRequest>>,
    pub secret_store: Arc<dyn SecretStore>,
    pub http_client: reqwest::Client,
}

#[derive(Clone)]
pub struct ChatState {
    pub inner: Arc<ChatStateInner>,
}

impl ChatState {
    pub fn new(secret_store: Arc<dyn SecretStore>) -> Self {
        let client = build_http_client().unwrap_or_else(|_| reqwest::Client::new());
        Self {
            inner: Arc::new(ChatStateInner {
                active: Mutex::new(None),
                secret_store,
                http_client: client,
            }),
        }
    }

    pub fn new_default() -> Self {
        Self::new(Arc::new(KeyringSecretStore))
    }

    /// Attempts to register an active request. Fails if another request is in progress.
    /// Returns a watch receiver to listen for cancellation.
    pub fn try_start_request(
        &self,
        request_id: &str,
        conversation_id: &str,
    ) -> Result<watch::Receiver<bool>, String> {
        let mut guard = self.inner.active.lock().map_err(|e| e.to_string())?;
        if let Some(existing) = guard.as_ref() {
            return Err(format!(
                "Another chat request ('{}') is currently active. Please wait or cancel it before starting a new request.",
                existing.request_id
            ));
        }

        let (tx, rx) = watch::channel(false);
        *guard = Some(ActiveRequest {
            request_id: request_id.to_string(),
            conversation_id: conversation_id.to_string(),
            cancel_tx: tx,
        });

        Ok(rx)
    }

    /// Cancels the active request if the requestId matches.
    pub fn cancel_request(&self, request_id: &str) -> bool {
        if let Ok(guard) = self.inner.active.lock() {
            if let Some(ref req) = *guard {
                if req.request_id == request_id {
                    let _ = req.cancel_tx.send(true);
                    return true;
                }
            }
        }
        false
    }

    /// Clears the active request guard if it matches the completed/failed requestId.
    pub fn finish_request(&self, request_id: &str) {
        if let Ok(mut guard) = self.inner.active.lock() {
            if let Some(ref req) = *guard {
                if req.request_id == request_id {
                    *guard = None;
                }
            }
        }
    }

    pub fn secret_store(&self) -> &Arc<dyn SecretStore> {
        &self.inner.secret_store
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.inner.http_client
    }
}
