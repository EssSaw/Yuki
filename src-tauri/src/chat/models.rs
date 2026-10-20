use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatSettingsDto {
    pub base_url: String,
    pub model: String,
    pub has_api_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatConfig {
    pub base_url: String,
    pub model: String,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:20128/v1".to_string(),
            model: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageStatus {
    Complete,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub status: MessageStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDetail {
    pub id: String,
    pub title: String,
    pub updated_at: String,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StartChatResponse {
    pub conversation_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolActivityStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivityPayload {
    pub id: String,
    pub name: String,
    pub status: ToolActivityStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatStreamPayload {
    pub request_id: String,
    pub conversation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<MessageStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_activity: Option<ToolActivityPayload>,
}

impl ChatStreamPayload {
    pub fn delta(request_id: String, conversation_id: String, delta: String) -> Self {
        Self {
            request_id,
            conversation_id,
            delta: Some(delta),
            done: None,
            error: None,
            status: None,
            tool_activity: None,
        }
    }

    pub fn terminal(
        request_id: String,
        conversation_id: String,
        status: MessageStatus,
        error: Option<String>,
    ) -> Self {
        Self {
            request_id,
            conversation_id,
            delta: None,
            done: Some(true),
            error,
            status: Some(status),
            tool_activity: None,
        }
    }

    pub fn activity(
        request_id: String,
        conversation_id: String,
        activity: ToolActivityPayload,
    ) -> Self {
        Self {
            request_id,
            conversation_id,
            delta: None,
            done: None,
            error: None,
            status: None,
            tool_activity: Some(activity),
        }
    }
}
