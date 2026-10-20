use serde::{Deserialize, Serialize};

use crate::mcp::models::McpEndpointGrant;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceSettings {
    pub data_root: Option<String>,
    pub project_roots: Vec<String>,
    pub provider_sharing_grants: Vec<String>,
    pub approved_mcp_endpoints: Vec<McpEndpointGrant>,
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
