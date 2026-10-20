use serde::{Deserialize, Serialize};

use crate::mcp::models::{McpEndpointGrant, McpExecutableGrant};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceSettings {
    pub data_root: Option<String>,
    pub project_roots: Vec<String>,
    pub provider_sharing_grants: Vec<String>,
    pub approved_mcp_endpoints: Vec<McpEndpointGrant>,
    #[serde(default)]
    pub approved_mcp_executables: Vec<McpExecutableGrant>,
    #[serde(default)]
    pub agent_write_roots: Vec<AgentWriteRootGrant>,
    #[serde(default)]
    pub agent_command_templates: Vec<AgentCommandTemplateGrant>,
    #[serde(default)]
    pub grant_generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentWriteRootGrant {
    pub id: String,
    pub canonical_path: String,
    pub project_root: String,
    pub created_at: String,
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCommandTemplateGrant {
    pub id: String,
    pub project_root: String,
    pub canonical_executable: String,
    pub args: Vec<String>,
    pub working_dir_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_sha256: Option<String>,
    pub created_at: String,
    pub generation: u64,
    pub execution_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPermissionSnapshot {
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_root: Option<String>,
    pub is_task_root_approved: bool,
    pub provider_sharing_enabled: bool,
    pub write_roots: Vec<AgentWriteRootGrant>,
    pub command_templates: Vec<AgentCommandTemplateGrant>,
    pub execution_available: bool,
    pub unsandboxed_disclosure: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentWriteRootResult {
    pub grant: AgentWriteRootGrant,
    pub snapshot: AgentPermissionSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCommandTemplateResult {
    pub grant: AgentCommandTemplateGrant,
    pub snapshot: AgentPermissionSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRevokeResult {
    pub revoked_id: String,
    pub snapshot: AgentPermissionSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEntry {
    pub name: String,
    pub relative_path: String,
    pub is_directory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileChunk {
    pub path: String,
    pub text: String,
    pub offset: u64,
    pub next_offset: u64,
    pub total_bytes: u64,
    pub eof: bool,
}
