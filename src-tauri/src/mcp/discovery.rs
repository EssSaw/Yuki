use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use crate::workspace::discovery::is_symlink_or_reparse;
use crate::workspace::explorer::validate_project_root;
use crate::workspace::settings::load_settings;

use super::models::{
    DOCUMENTED_DEFAULT_MCP_ENDPOINT, MAX_MCP_CONFIG_FILE_SIZE, McpCandidate, McpCandidateSource,
    McpDiscoveryResult, McpEndpointGrant, McpIdentityStatus, McpReachability, ProjectRelation,
    REACHABILITY_PROBE_TIMEOUT_MS,
};
use super::url::validate_and_canonicalize_loopback_url;

pub struct ConfigTarget {
    pub dir: Option<&'static str>,
    pub file: &'static str,
    pub rel_display: &'static str,
}

pub const TARGET_CONFIGS: &[ConfigTarget] = &[
    ConfigTarget {
        dir: None,
        file: ".mcp.json",
        rel_display: ".mcp.json",
    },
    ConfigTarget {
        dir: Some(".vscode"),
        file: "mcp.json",
        rel_display: ".vscode/mcp.json",
    },
    ConfigTarget {
        dir: Some(".cursor"),
        file: "mcp.json",
        rel_display: ".cursor/mcp.json",
    },
];

/// Inspects approved project roots and builds the full candidate inventory.
/// If `probe` is true, performs bounded short reachability probes for supported endpoints.
pub async fn discover_mcp_candidates_core(
    config_dir: &Path,
    probe: bool,
) -> Result<McpDiscoveryResult, String> {
    let settings = load_settings(config_dir)?;
    let approved_grants = &settings.approved_mcp_endpoints;

    let mut candidates = Vec::new();
    let mut scanned_roots = Vec::new();
    let mut configs_inspected = Vec::new();
    let mut warnings = Vec::new();

    // 1. Always include the documented default candidate: http://localhost:8080/mcp
    let default_canonical = DOCUMENTED_DEFAULT_MCP_ENDPOINT.to_string();
    let default_is_approved = approved_grants
        .iter()
        .any(|g| g.canonical_url == default_canonical);

    candidates.push(McpCandidate {
        id: "default:unity-mcp:http://localhost:8080/mcp".to_string(),
        server_name: "unity-mcp (documented default)".to_string(),
        endpoint_url: Some(default_canonical),
        transport: "http".to_string(),
        source: McpCandidateSource::DocumentedDefault {
            default_url: DOCUMENTED_DEFAULT_MCP_ENDPOINT.to_string(),
        },
        project_relation: ProjectRelation::ExplicitUnverified {
            details: "Documented default endpoint has no inherent association with any specific project"
                .to_string(),
        },
        is_approved: default_is_approved,
        supported: true,
        reachability: McpReachability::NotProbed,
        identity_status: McpIdentityStatus::Unverified {
            details: "Default candidate is a documented endpoint; server running state and Unity editor identity are unverified"
                .to_string(),
        },
        warnings: Vec::new(),
    });

    // 2. Inspect approved project roots
    for root_str in &settings.project_roots {
        let root_path = Path::new(root_str);
        let canonical_root = match validate_project_root(root_path) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!("Skipping invalid project root '{root_str}': {e}"));
                continue;
            }
        };

        scanned_roots.push(canonical_root.to_string_lossy().to_string());

        for target in TARGET_CONFIGS {
            inspect_target_config(
                &canonical_root,
                target,
                approved_grants,
                &mut candidates,
                &mut configs_inspected,
                &mut warnings,
            );
        }
    }

    // 3. If probing requested, probe unique candidate endpoints
    if probe {
        probe_candidates(&mut candidates).await;
    }

    Ok(McpDiscoveryResult {
        candidates,
        scanned_roots,
        configs_inspected,
        warnings,
    })
}

fn inspect_target_config(
    canonical_root: &Path,
    target: &ConfigTarget,
    approved_grants: &[McpEndpointGrant],
    candidates: &mut Vec<McpCandidate>,
    configs_inspected: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let full_path = match target.dir {
        Some(dir_name) => {
            let dir_path = canonical_root.join(dir_name);
            if !dir_path.exists() {
                return;
            }
            if is_symlink_or_reparse(&dir_path) {
                warnings.push(format!(
                    "Skipped directory '{}' in '{}': symlinks and junctions are forbidden",
                    dir_name,
                    canonical_root.display()
                ));
                return;
            }
            if let Ok(canon_dir) = dunce::canonicalize(&dir_path)
                && !canon_dir.starts_with(canonical_root)
            {
                warnings.push(format!(
                    "Directory '{}' in '{}' escapes project root boundary",
                    dir_name,
                    canonical_root.display()
                ));
                return;
            }
            dir_path.join(target.file)
        }
        None => canonical_root.join(target.file),
    };

    if !full_path.exists() {
        return;
    }

    // Check symlink/reparse point on file
    if is_symlink_or_reparse(&full_path) {
        warnings.push(format!(
            "Skipped config file '{}' in '{}': symlinks and junctions are forbidden",
            target.rel_display,
            canonical_root.display()
        ));
        return;
    }

    // Check metadata
    let meta = match std::fs::symlink_metadata(&full_path) {
        Ok(m) => m,
        Err(e) => {
            warnings.push(format!(
                "Failed to read metadata for '{}' in '{}': {e}",
                target.rel_display,
                canonical_root.display()
            ));
            return;
        }
    };

    if !meta.is_file() {
        return;
    }

    // Enforce size limit
    if meta.len() > MAX_MCP_CONFIG_FILE_SIZE {
        warnings.push(format!(
            "Config file '{}' in '{}' exceeds maximum allowed size ({} bytes > {} bytes)",
            target.rel_display,
            canonical_root.display(),
            meta.len(),
            MAX_MCP_CONFIG_FILE_SIZE
        ));
        return;
    }

    // Verify canonical boundary
    if let Ok(canon_file) = dunce::canonicalize(&full_path)
        && !canon_file.starts_with(canonical_root)
    {
        warnings.push(format!(
            "Config file '{}' escapes project root '{}'",
            target.rel_display,
            canonical_root.display()
        ));
        return;
    }

    // Read bounded UTF-8
    let mut file = match File::open(&full_path) {
        Ok(f) => f,
        Err(e) => {
            warnings.push(format!(
                "Failed to open config file '{}' in '{}': {e}",
                target.rel_display,
                canonical_root.display()
            ));
            return;
        }
    };

    let mut buffer = Vec::new();
    if let Err(e) = file.read_to_end(&mut buffer) {
        warnings.push(format!(
            "Failed to read config file '{}' in '{}': {e}",
            target.rel_display,
            canonical_root.display()
        ));
        return;
    }

    if buffer.contains(&0) {
        warnings.push(format!(
            "Config file '{}' in '{}' contains binary data and cannot be parsed",
            target.rel_display,
            canonical_root.display()
        ));
        return;
    }

    let content = match String::from_utf8(buffer) {
        Ok(s) => s,
        Err(_) => {
            warnings.push(format!(
                "Config file '{}' in '{}' is not valid UTF-8",
                target.rel_display,
                canonical_root.display()
            ));
            return;
        }
    };

    let json_val: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(format!(
                "Failed to parse JSON in '{}' in '{}': {e}",
                target.rel_display,
                canonical_root.display()
            ));
            return;
        }
    };

    configs_inspected.push(format!(
        "{}/{}",
        canonical_root.display(),
        target.rel_display
    ));

    // Look for mcpServers or servers
    let mut found_servers = false;

    if let Some(servers_obj) = json_val.get("mcpServers").and_then(|v| v.as_object()) {
        found_servers = true;
        parse_servers_map(
            canonical_root,
            target.rel_display,
            servers_obj,
            approved_grants,
            candidates,
            warnings,
        );
    }

    if let Some(servers_obj) = json_val.get("servers").and_then(|v| v.as_object()) {
        found_servers = true;
        parse_servers_map(
            canonical_root,
            target.rel_display,
            servers_obj,
            approved_grants,
            candidates,
            warnings,
        );
    }

    if !found_servers {
        warnings.push(format!(
            "Config file '{}' in '{}' does not contain 'mcpServers' or 'servers' object",
            target.rel_display,
            canonical_root.display()
        ));
    }
}

fn parse_servers_map(
    canonical_root: &Path,
    rel_str: &str,
    servers: &serde_json::Map<String, serde_json::Value>,
    approved_grants: &[McpEndpointGrant],
    candidates: &mut Vec<McpCandidate>,
    warnings: &mut Vec<String>,
) {
    let root_str = canonical_root.to_string_lossy().to_string();

    for (server_name, server_val) in servers {
        let server_obj = match server_val.as_object() {
            Some(obj) => obj,
            None => {
                warnings.push(format!(
                    "Server entry '{server_name}' in '{rel_str}' is not an object"
                ));
                continue;
            }
        };

        // Check if stdio transport: has "command"
        if server_obj.contains_key("command") {
            // stdio transport: never execute command, never extract env/headers/tokens
            let candidate_id = format!("config:{}:{}:{}", root_str, rel_str, server_name);
            candidates.push(McpCandidate {
                id: candidate_id,
                server_name: server_name.clone(),
                endpoint_url: None,
                transport: "stdio (unsupported)".to_string(),
                source: McpCandidateSource::ProjectConfig {
                    project_root: root_str.clone(),
                    config_path: rel_str.to_string(),
                    server_name: server_name.clone(),
                },
                project_relation: ProjectRelation::ExplicitUnverified {
                    details: format!(
                        "Configured in project root '{}' at '{}' as stdio transport; runtime editor binding unverified",
                        root_str, rel_str
                    ),
                },
                is_approved: false,
                supported: false,
                reachability: McpReachability::Unsupported {
                    reason: "Stdio transport is not supported; commands are never executed".to_string(),
                },
                identity_status: McpIdentityStatus::Unverified {
                    details: "Stdio configuration commands are not executed".to_string(),
                },
                warnings: vec![
                    "Stdio transport commands are not executed for security; only loopback HTTP is supported"
                        .to_string(),
                ],
            });
            continue;
        }

        // Check if URL is present
        if let Some(url_str) = server_obj.get("url").and_then(|u| u.as_str()) {
            match validate_and_canonicalize_loopback_url(url_str) {
                Ok(canonical_url) => {
                    let is_approved = approved_grants
                        .iter()
                        .any(|g| g.canonical_url == canonical_url);
                    let candidate_id = format!("config:{}:{}:{}", root_str, rel_str, server_name);

                    candidates.push(McpCandidate {
                        id: candidate_id,
                        server_name: server_name.clone(),
                        endpoint_url: Some(canonical_url),
                        transport: "http".to_string(),
                        source: McpCandidateSource::ProjectConfig {
                            project_root: root_str.clone(),
                            config_path: rel_str.to_string(),
                            server_name: server_name.clone(),
                        },
                        project_relation: ProjectRelation::ExplicitUnverified {
                            details: format!(
                                "Declared in project root '{}' at '{}'; runtime editor instance and project binding are unverified",
                                root_str, rel_str
                            ),
                        },
                        is_approved,
                        supported: true,
                        reachability: McpReachability::NotProbed,
                        identity_status: McpIdentityStatus::Unverified {
                            details: "Candidate endpoint discovery is not proof of Unity editor connection or CoplayDev server identity"
                                .to_string(),
                        },
                        warnings: Vec::new(),
                    });
                }
                Err(err) => {
                    let candidate_id = format!("config:{}:{}:{}", root_str, rel_str, server_name);
                    candidates.push(McpCandidate {
                        id: candidate_id,
                        server_name: server_name.clone(),
                        endpoint_url: None,
                        transport: "http (invalid)".to_string(),
                        source: McpCandidateSource::ProjectConfig {
                            project_root: root_str.clone(),
                            config_path: rel_str.to_string(),
                            server_name: server_name.clone(),
                        },
                        project_relation: ProjectRelation::ExplicitUnverified {
                            details: format!(
                                "Declared in project root '{}' at '{}' with invalid URL",
                                root_str, rel_str
                            ),
                        },
                        is_approved: false,
                        supported: false,
                        reachability: McpReachability::Unsupported {
                            reason: format!("Invalid URL: {err}"),
                        },
                        identity_status: McpIdentityStatus::Unverified {
                            details: "Invalid candidate URL".to_string(),
                        },
                        warnings: vec![format!("Rejected candidate '{server_name}': {err}")],
                    });
                }
            }
            continue;
        }

        warnings.push(format!(
            "Server entry '{server_name}' in '{rel_str}' has neither 'url' nor 'command'"
        ));
    }
}

/// Probes reachability for supported candidate endpoints.
/// Captures HTTP status code only, without sending MCP initialize or requesting tools/resources.
/// Redirects are strictly disabled.
async fn probe_candidates(candidates: &mut [McpCandidate]) {
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(REACHABILITY_PROBE_TIMEOUT_MS))
        .connect_timeout(Duration::from_millis(REACHABILITY_PROBE_TIMEOUT_MS))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            for c in candidates.iter_mut() {
                if c.supported && c.endpoint_url.is_some() {
                    c.reachability = McpReachability::Unreachable {
                        reason: format!("Failed to create HTTP client for probe: {e}"),
                    };
                }
            }
            return;
        }
    };

    // Collect unique URLs to probe
    let mut unique_urls = HashSet::new();
    for c in candidates.iter() {
        if c.supported
            && let Some(ref url) = c.endpoint_url
        {
            unique_urls.insert(url.clone());
        }
    }

    let mut results: HashMap<String, McpReachability> = HashMap::new();
    for url in unique_urls {
        let reachability = probe_single_endpoint(&client, &url).await;
        results.insert(url, reachability);
    }

    // Apply probe results back to candidates
    for c in candidates.iter_mut() {
        if c.supported
            && let Some(ref url) = c.endpoint_url
            && let Some(reachability) = results.get(url)
        {
            c.reachability = reachability.clone();
        }
        // Identity status remains explicitly Unverified
        c.identity_status = McpIdentityStatus::Unverified {
            details: "HTTP status confirms transport listener availability only; server identity, Unity editor connection, and CoplayDev adapter are unverified"
                .to_string(),
        };
    }
}

pub async fn probe_single_endpoint(client: &reqwest::Client, url: &str) -> McpReachability {
    match client.get(url).send().await {
        Ok(resp) => McpReachability::Reachable {
            status_code: resp.status().as_u16(),
        },
        Err(e) => {
            let reason = if e.is_timeout() {
                format!("Request timed out ({REACHABILITY_PROBE_TIMEOUT_MS}ms)")
            } else if e.is_connect() {
                "Connection refused (no server listening)".to_string()
            } else {
                format!("HTTP probe failed: {e}")
            };
            McpReachability::Unreachable { reason }
        }
    }
}
