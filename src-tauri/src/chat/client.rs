use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::models::{ChatMessage, MessageRole, MessageStatus};

pub const MAX_INPUT_CHARS: usize = 32_000;
pub const MAX_HISTORY_CHARS: usize = 16_000;
pub const MAX_STREAM_BYTES: usize = 5 * 1024 * 1024; // 5 MB

/// Validates that an endpoint URL is valid and conforms to security rules:
/// - Plain HTTP is ONLY permitted for loopback addresses (localhost, 127.0.0.1, ::1).
/// - Remote endpoints MUST use HTTPS.
pub fn validate_endpoint(url_str: &str) -> Result<url::Url, String> {
    let trimmed = url_str.trim();
    if trimmed.is_empty() {
        return Err("Endpoint URL cannot be empty.".to_string());
    }

    let parsed =
        url::Url::parse(trimmed).map_err(|e| format!("Invalid endpoint URL format: {e}"))?;

    let scheme = parsed.scheme();
    if scheme == "http" {
        let host = parsed.host_str().unwrap_or("");
        let is_loopback = host.eq_ignore_ascii_case("localhost")
            || host == "127.0.0.1"
            || host == "::1"
            || host == "[::1]";
        if !is_loopback {
            return Err(
                "HTTP is only permitted for loopback addresses (localhost, 127.0.0.1, ::1). Remote endpoints must use HTTPS."
                    .to_string(),
            );
        }
    } else if scheme != "https" {
        return Err(format!(
            "Unsupported URL scheme '{scheme}'. Only HTTP (loopback) and HTTPS are permitted."
        ));
    }

    Ok(parsed)
}

/// Creates a reqwest client configured with no redirect policy (to prevent leaking API keys)
/// and sensible connection timeouts.
pub fn build_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenAiMessage {
    pub role: &'static str,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ApiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl From<OpenAiMessage> for ApiMessage {
    fn from(m: OpenAiMessage) -> Self {
        Self {
            role: m.role.to_string(),
            content: Some(m.content),
            tool_calls: None,
            tool_call_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ApiFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<super::tools::ToolDefinition>>,
    pub stream: bool,
}

/// Prepares a bounded context of recent completed messages within the specified character budget.
/// Preserves the original history on disk; only bounds what is transmitted.
/// Never sends tools or files.
pub fn prepare_bounded_messages(
    messages: &[ChatMessage],
    max_history_chars: usize,
) -> Vec<OpenAiMessage> {
    let mut selected = Vec::new();
    let mut current_chars = 0;

    // Walk backwards from newest to oldest
    for msg in messages.iter().rev() {
        if msg.status != MessageStatus::Complete {
            continue;
        }
        let len = msg.content.len();
        if current_chars + len > max_history_chars && !selected.is_empty() {
            break;
        }
        let role = match msg.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };
        selected.push(OpenAiMessage {
            role,
            content: msg.content.clone(),
        });
        current_chars += len;
    }

    selected.reverse();
    selected
}

#[derive(Deserialize)]
struct OpenAiModelsResponse {
    #[serde(default)]
    data: Vec<OpenAiModelItem>,
}

#[derive(Deserialize)]
struct OpenAiModelItem {
    id: String,
}

/// Fetches available models from the OpenAI-compatible `/models` endpoint.
/// Validates endpoint and sanitizes any network errors.
pub async fn fetch_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>, String> {
    validate_endpoint(base_url)?;

    let base = base_url.trim_end_matches('/');
    let models_url = format!("{base}/models");

    let mut req = client.get(&models_url).timeout(Duration::from_secs(15));

    if let Some(key) = api_key {
        let trimmed_key = key.trim();
        if !trimmed_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {trimmed_key}"));
        }
    }

    let resp = req.send().await.map_err(|e| {
        if e.is_connect() {
            format!("Failed to connect to 9router at '{base}'. Ensure the service is running.")
        } else if e.is_timeout() {
            "Connection to 9router timed out.".to_string()
        } else {
            "Network error while connecting to 9router.".to_string()
        }
    })?;

    let status = resp.status();
    if !status.is_success() {
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(
                "Authentication failed (HTTP 401). Please check your API key in settings."
                    .to_string(),
            );
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(format!(
                "Models endpoint not found (HTTP 404) at '{models_url}'."
            ));
        }
        return Err(format!("Provider returned HTTP status {status}."));
    }

    let data = resp
        .json::<OpenAiModelsResponse>()
        .await
        .map_err(|e| format!("Failed to parse models list from provider: {e}"))?;

    let mut models: Vec<String> = data.data.into_iter().map(|m| m.id).collect();
    models.sort();
    models.dedup();

    Ok(models)
}
