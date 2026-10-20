use std::path::{Path, PathBuf};

use crate::workspace::discovery::is_symlink_or_reparse;
use crate::workspace::settings::{load_settings, save_settings_atomic};

use super::models::{
    McpEndpointGrant, McpExecutableGrant, McpExecutableGrantResult, McpGrantResult,
};
use super::url::validate_and_canonicalize_loopback_url;

/// Approves a canonical loopback MCP endpoint.
/// Persists the grant in app config settings (separate from provider sharing).
pub fn approve_endpoint_impl(config_dir: &Path, endpoint: &str) -> Result<McpGrantResult, String> {
    let canonical = validate_and_canonicalize_loopback_url(endpoint)?;
    let _guard = crate::workspace::settings::SETTINGS_MUTATION_LOCK
        .lock()
        .map_err(|e| format!("Failed to acquire settings mutation lock: {e}"))?;
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
    let _guard = crate::workspace::settings::SETTINGS_MUTATION_LOCK
        .lock()
        .map_err(|e| format!("Failed to acquire settings mutation lock: {e}"))?;
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

/// Validates that an executable path is an absolute, existing normal file on disk,
/// rejecting any symlinks or NTFS reparse points/junctions, and verifying .exe extension on Windows.
pub fn validate_and_canonicalize_executable(path_str: &str) -> Result<PathBuf, String> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return Err("Executable path cannot be empty".to_string());
    }

    let p = Path::new(trimmed);
    if !p.is_absolute() {
        return Err(format!(
            "Executable path '{trimmed}' must be an absolute path"
        ));
    }

    if !p.exists() {
        return Err(format!(
            "Executable file '{trimmed}' does not exist on disk"
        ));
    }

    // Check symlink or reparse point before canonicalization
    if is_symlink_or_reparse(p) {
        return Err(format!(
            "Executable path '{trimmed}' is a symlink or reparse point; links and junctions are strictly forbidden"
        ));
    }

    let canonical = dunce::canonicalize(p)
        .map_err(|e| format!("Failed to canonicalize executable path '{trimmed}': {e}"))?;

    if !canonical.is_file() {
        return Err(format!(
            "Path '{}' is not a regular file",
            canonical.display()
        ));
    }

    // Check canonical file for symlinks or reparse points
    if is_symlink_or_reparse(&canonical) {
        return Err(format!(
            "Canonical executable path '{}' is a symlink or reparse point",
            canonical.display()
        ));
    }

    #[cfg(windows)]
    {
        let ext = canonical.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("exe") {
            return Err(format!(
                "Executable file '{}' must have a .exe extension on Windows",
                canonical.display()
            ));
        }
    }

    Ok(canonical)
}

/// Checks that an executable path is currently approved in workspace settings.
pub fn ensure_executable_is_approved(
    config_dir: &Path,
    canonical_path: &str,
) -> Result<(), String> {
    let settings = load_settings(config_dir)?;
    let is_approved = settings
        .approved_mcp_executables
        .iter()
        .any(|g| g.canonical_path == canonical_path);
    if !is_approved {
        return Err(format!(
            "MCP executable '{canonical_path}' is not approved or approval grant has been revoked."
        ));
    }
    Ok(())
}

/// Approves an exact canonical local executable.
/// Persists the grant in app config settings (separate from loopback endpoints and provider sharing).
pub fn approve_executable_impl(
    config_dir: &Path,
    path_str: &str,
) -> Result<McpExecutableGrantResult, String> {
    let canonical = validate_and_canonicalize_executable(path_str)?;
    let canonical_str = canonical.to_string_lossy().to_string();
    let _guard = crate::workspace::settings::SETTINGS_MUTATION_LOCK
        .lock()
        .map_err(|e| format!("Failed to acquire settings mutation lock: {e}"))?;
    let mut settings = load_settings(config_dir)?;

    let is_already_approved = settings
        .approved_mcp_executables
        .iter()
        .any(|g| g.canonical_path == canonical_str);

    if !is_already_approved {
        let now = chrono::Utc::now().to_rfc3339();
        settings.approved_mcp_executables.push(McpExecutableGrant {
            canonical_path: canonical_str.clone(),
            approved_at: now,
        });
        save_settings_atomic(config_dir, &settings)?;
    }

    Ok(McpExecutableGrantResult {
        canonical_path: canonical_str,
        is_approved: true,
        approved_executables: settings.approved_mcp_executables,
    })
}

/// Revokes approval for an exact canonical local executable.
/// Removes the grant from app config settings.
pub fn revoke_executable_impl(
    config_dir: &Path,
    path_str: &str,
) -> Result<McpExecutableGrantResult, String> {
    let canonical_str = if let Ok(canon) = validate_and_canonicalize_executable(path_str) {
        canon.to_string_lossy().to_string()
    } else {
        path_str.trim().to_string()
    };

    let _guard = crate::workspace::settings::SETTINGS_MUTATION_LOCK
        .lock()
        .map_err(|e| format!("Failed to acquire settings mutation lock: {e}"))?;
    let mut settings = load_settings(config_dir)?;

    let was_present = settings
        .approved_mcp_executables
        .iter()
        .any(|g| g.canonical_path == canonical_str);

    if was_present {
        settings
            .approved_mcp_executables
            .retain(|g| g.canonical_path != canonical_str);
        save_settings_atomic(config_dir, &settings)?;
    }

    Ok(McpExecutableGrantResult {
        canonical_path: canonical_str,
        is_approved: false,
        approved_executables: settings.approved_mcp_executables,
    })
}

/// Returns all currently approved MCP executable grants.
pub fn get_mcp_executable_grants_impl(
    config_dir: &Path,
) -> Result<Vec<McpExecutableGrant>, String> {
    let settings = load_settings(config_dir)?;
    Ok(settings.approved_mcp_executables)
}
