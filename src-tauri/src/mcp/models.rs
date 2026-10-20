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
