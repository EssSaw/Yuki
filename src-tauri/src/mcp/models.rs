use serde::{Deserialize, Serialize};

/// Documented default endpoint for CoplayDev/unity-mcp HTTP transport.
pub const DOCUMENTED_DEFAULT_MCP_ENDPOINT: &str = "http://localhost:8080/mcp";

/// Maximum allowed configuration file size (64 KiB).
pub const MAX_MCP_CONFIG_FILE_SIZE: u64 = 64 * 1024;

/// Reachability probe timeout in milliseconds.
pub const REACHABILITY_PROBE_TIMEOUT_MS: u64 = 1500;

/// Persisted user approval grant for a canonical loopback MCP endpoint.
/// Completely separate from provider-sharing grants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct McpEndpointGrant {
    pub canonical_url: String,
    pub approved_at: String,
}

/// Source of an MCP candidate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum McpCandidateSource {
    DocumentedDefault {
        default_url: String,
    },
    ProjectConfig {
        project_root: String,
        config_path: String,
        server_name: String,
    },
}

/// Explicit candidate relation to project.
/// Always marked unverified to avoid confusing configuration presence with verified editor binding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ProjectRelation {
    ExplicitUnverified { details: String },
}

/// Reachability probe status of an endpoint.
/// Reports HTTP transport listener availability only, not application-level identity or auth success.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum McpReachability {
    NotProbed,
    Reachable { status_code: u16 },
    Unreachable { reason: String },
    Unsupported { reason: String },
}

/// Explicit verification status of server identity.
/// Separate from reachability: an HTTP response does not prove Unity or CoplayDev identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum McpIdentityStatus {
    Unverified { details: String },
}

/// Discovered MCP server candidate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpCandidate {
    pub id: String,
    pub server_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
    pub transport: String,
    pub source: McpCandidateSource,
    pub project_relation: ProjectRelation,
    pub is_approved: bool,
    pub supported: bool,
    pub reachability: McpReachability,
    pub identity_status: McpIdentityStatus,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// Aggregate discovery result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpDiscoveryResult {
    pub candidates: Vec<McpCandidate>,
    pub scanned_roots: Vec<String>,
    pub configs_inspected: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// Result of an endpoint grant approval or revocation action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpGrantResult {
    pub canonical_url: String,
    pub is_approved: bool,
    pub approved_endpoints: Vec<McpEndpointGrant>,
}

/// Supported MCP protocol versions in order of negotiation preference.
pub const SUPPORTED_MCP_PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-11-25"];

/// Default client MCP protocol version to propose.
pub const DEFAULT_MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Maximum number of concurrent active MCP sessions in memory.
pub const MAX_ACTIVE_MCP_SESSIONS: usize = 5;

/// MCP request timeout in milliseconds (5 seconds).
pub const MCP_REQUEST_TIMEOUT_MS: u64 = 5000;

/// Maximum allowed HTTP response size for MCP requests (1 MiB).
pub const MAX_MCP_RESPONSE_BYTES: usize = 1024 * 1024;

/// Connection result returned to UI after successfully initializing an approved MCP endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpConnectionResult {
    pub session_id: String,
    pub endpoint_url: String,
    pub protocol_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
}

/// Information about a connected Unity editor instance discovered via `mcpforunity://instances`.
/// Identified strictly by exact `Name@hash`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnityInstanceInfo {
    pub id: String,
    pub name: String,
    pub hash: String,
    #[serde(default)]
    pub is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// Local verification status of the bound Unity instance project root.
/// Project root must canonicalize and match an approved workspace project root.
/// Cannot be inferred from project name alone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UnityProjectVerificationStatus {
    Verified {
        canonical_root: String,
        matched_approved_root: String,
        provider_sharing_enabled: bool,
    },
    Mismatched {
        reported_root: String,
        reason: String,
    },
    Unverifiable {
        reason: String,
    },
}

/// Sanitized bound Unity editor state returned to local UI.
/// Never forwarded to external AI providers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BoundUnityState {
    pub session_id: String,
    pub bound_instance_id: String,
    pub endpoint_url: String,
    pub is_playing: bool,
    pub is_paused: bool,
    pub is_compiling: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unity_version: Option<String>,
    pub project_verification: UnityProjectVerificationStatus,
    pub provider_sharing_eligible: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// Sanitized result of disconnecting an MCP session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpDisconnectResult {
    pub session_id: String,
    pub success: bool,
    pub message: String,
}

/// Persisted user approval grant for an exact canonical local executable.
/// Completely separate from loopback endpoint grants and provider-sharing grants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct McpExecutableGrant {
    pub canonical_path: String,
    pub approved_at: String,
}

/// Result of an executable grant approval or revocation action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpExecutableGrantResult {
    pub canonical_path: String,
    pub is_approved: bool,
    pub approved_executables: Vec<McpExecutableGrant>,
}

/// Connection result returned after successfully launching and initializing an approved MCP stdio executable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpStdioConnectionResult {
    pub session_id: String,
    pub canonical_path: String,
    pub transport: String,
    pub protocol_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    pub process_id: u32,
}

/// Bounded metadata for a tool discovered over MCP.
/// Exposed for future version-reviewed allowlist; not an authorization for execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpToolMetadata {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
}

/// Bounded metadata for a resource discovered over MCP.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceMetadata {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// Explicit status of official editor binding.
/// Clearly indicates if binding is pending editor Project Settings approval or undocumented,
/// preventing false claims of verified project identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum McpEditorBindingStatus {
    PendingOrUndocumented { details: String },
}

/// Aggregate capability inventory returned to local caller after querying an initialized stdio session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpStdioCapabilities {
    pub session_id: String,
    pub canonical_path: String,
    pub protocol_version: String,
    pub tools: Vec<McpToolMetadata>,
    pub resources: Vec<McpResourceMetadata>,
    pub editor_binding_status: McpEditorBindingStatus,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}
