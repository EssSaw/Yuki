use std::path::Path;

use crate::workspace::settings::{load_settings, save_settings_atomic};

use super::models::{McpEndpointGrant, McpGrantResult};
use super::url::validate_and_canonicalize_loopback_url;

/// Approves a canonical loopback MCP endpoint.
/// Persists the grant in app config settings (separate from provider sharing).
pub fn approve_endpoint_impl(config_dir: &Path, endpoint: &str) -> Result<McpGrantResult, String> {
    let canonical = validate_and_canonicalize_loopback_url(endpoint)?;
    let mut settings = load_settings(config_dir)?;

    let is_already_approved = settings
        .approved_mcp_endpoints
        .iter()
        .any(|g| g.canonical_url == canonical);

    if !is_already_approved {
        let now = chrono::Utc::now().to_rfc3339();
        settings.approved_mcp_endpoints.push(McpEndpointGrant {
            canonical_url: canonical.clone(),
            approved_at: now,
        });
        save_settings_atomic(config_dir, &settings)?;
    }

    Ok(McpGrantResult {
        canonical_url: canonical,
        is_approved: true,
        approved_endpoints: settings.approved_mcp_endpoints,
    })
}

/// Revokes approval for a canonical loopback MCP endpoint.
/// Removes the grant from app config settings.
pub fn revoke_endpoint_impl(config_dir: &Path, endpoint: &str) -> Result<McpGrantResult, String> {
    let canonical = validate_and_canonicalize_loopback_url(endpoint)?;
    let mut settings = load_settings(config_dir)?;

    let was_present = settings
        .approved_mcp_endpoints
        .iter()
        .any(|g| g.canonical_url == canonical);

    if was_present {
        settings
            .approved_mcp_endpoints
            .retain(|g| g.canonical_url != canonical);
        save_settings_atomic(config_dir, &settings)?;
    }

    Ok(McpGrantResult {
        canonical_url: canonical,
        is_approved: false,
        approved_endpoints: settings.approved_mcp_endpoints,
    })
}

/// Returns all currently approved MCP endpoint grants.
pub fn get_mcp_grants_impl(config_dir: &Path) -> Result<Vec<McpEndpointGrant>, String> {
    let settings = load_settings(config_dir)?;
    Ok(settings.approved_mcp_endpoints)
}
