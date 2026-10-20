use std::io::Write;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::discovery::{discover_mcp_candidates_core, probe_single_endpoint};
use super::grants::{approve_endpoint_impl, get_mcp_grants_impl, revoke_endpoint_impl};
use super::models::{
    DOCUMENTED_DEFAULT_MCP_ENDPOINT, MAX_MCP_CONFIG_FILE_SIZE, McpIdentityStatus, McpReachability,
};
use super::url::validate_and_canonicalize_loopback_url;
use crate::workspace::models::WorkspaceSettings;
use crate::workspace::settings::save_settings_atomic;

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(name: &str) -> Self {
        let unique = format!(
            "{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let base = dunce::canonicalize(std::env::current_dir().unwrap())
            .unwrap()
            .join("target")
            .join("test_temp")
            .join(unique);
        std::fs::create_dir_all(&base).unwrap();
        Self { root: base }
    }

    fn config_dir(&self) -> PathBuf {
        let dir = self.root.join("config");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn new_project(&self, name: &str) -> PathBuf {
        let dir = self.root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dunce::canonicalize(&dir).unwrap()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_loopback_url_validation_success() {
    assert_eq!(
        validate_and_canonicalize_loopback_url("http://localhost:8080/mcp").unwrap(),
        "http://localhost:8080/mcp"
    );
    assert_eq!(
        validate_and_canonicalize_loopback_url("http://localhost:8080/mcp/").unwrap(),
        "http://localhost:8080/mcp"
    );
    assert_eq!(
        validate_and_canonicalize_loopback_url("http://127.0.0.1:8080/mcp").unwrap(),
        "http://127.0.0.1:8080/mcp"
    );
    assert_eq!(
        validate_and_canonicalize_loopback_url("http://[::1]:8080/mcp").unwrap(),
        "http://[::1]:8080/mcp"
    );
    assert_eq!(
        validate_and_canonicalize_loopback_url("http://localhost:8080").unwrap(),
        "http://localhost:8080/"
    );
    assert_eq!(
        validate_and_canonicalize_loopback_url("http://localhost:8080/").unwrap(),
        "http://localhost:8080/"
    );
    assert_eq!(
        validate_and_canonicalize_loopback_url("http://LOCALHOST:8080/mcp").unwrap(),
        "http://localhost:8080/mcp"
    );
    assert_eq!(
        validate_and_canonicalize_loopback_url("http://localhost").unwrap(),
        "http://localhost:80/"
    );
}

#[test]
fn test_loopback_url_rejections_security() {
    // Non-loopback host names / misleading localhost suffixes
    assert!(
        validate_and_canonicalize_loopback_url("http://localhost.attacker.com:8080/mcp").is_err()
    );
    assert!(
        validate_and_canonicalize_loopback_url("http://localhost.localdomain:8080/mcp").is_err()
    );
    assert!(validate_and_canonicalize_loopback_url("http://evil-localhost:8080/mcp").is_err());
    assert!(validate_and_canonicalize_loopback_url("http://notlocalhost:8080/mcp").is_err());
    assert!(validate_and_canonicalize_loopback_url("http://127.0.0.1.nip.io:8080/mcp").is_err());
    assert!(validate_and_canonicalize_loopback_url("http://example.com/mcp").is_err());
    assert!(validate_and_canonicalize_loopback_url("http://unity.local:8080/mcp").is_err());

    // Non-loopback IPs
    assert!(validate_and_canonicalize_loopback_url("http://0.0.0.0:8080/mcp").is_err());
    assert!(validate_and_canonicalize_loopback_url("http://192.168.1.1:8080/mcp").is_err());
    assert!(validate_and_canonicalize_loopback_url("http://10.0.0.1:8080/mcp").is_err());
    assert!(validate_and_canonicalize_loopback_url("http://127.0.0.2:8080/mcp").is_err());
    assert!(validate_and_canonicalize_loopback_url("http://[::2]:8080/mcp").is_err());

    // Scheme != http
    assert!(validate_and_canonicalize_loopback_url("https://localhost:8080/mcp").is_err());
    assert!(validate_and_canonicalize_loopback_url("ftp://localhost:8080/mcp").is_err());
    assert!(validate_and_canonicalize_loopback_url("ws://localhost:8080/mcp").is_err());

    // Credentials
    assert!(validate_and_canonicalize_loopback_url("http://user:pass@localhost:8080/mcp").is_err());
    assert!(validate_and_canonicalize_loopback_url("http://admin@localhost:8080/mcp").is_err());

    // Query parameters
    assert!(validate_and_canonicalize_loopback_url("http://localhost:8080/mcp?token=xyz").is_err());
    assert!(validate_and_canonicalize_loopback_url("http://localhost:8080/mcp?").is_err());

    // Fragment
    assert!(validate_and_canonicalize_loopback_url("http://localhost:8080/mcp#anchor").is_err());

    // Invalid port
    assert!(validate_and_canonicalize_loopback_url("http://localhost:0/mcp").is_err());

    // Empty
    assert!(validate_and_canonicalize_loopback_url("").is_err());
    assert!(validate_and_canonicalize_loopback_url("   ").is_err());
}

#[tokio::test]
async fn test_json_formats_and_common_structures() {
    let env = TestEnv::new("test_json_formats");
    let cfg = env.config_dir();
    let proj = env.new_project("game_proj");

    // 1. .mcp.json with "mcpServers"
    let mcp_json = proj.join(".mcp.json");
    std::fs::write(
        &mcp_json,
        r#"{
            "mcpServers": {
                "unity-root": {
                    "url": "http://localhost:8080/mcp"
                }
            }
        }"#,
    )
    .unwrap();

    // 2. .vscode/mcp.json with "servers"
    let vscode_dir = proj.join(".vscode");
    std::fs::create_dir_all(&vscode_dir).unwrap();
    let vscode_mcp = vscode_dir.join("mcp.json");
    std::fs::write(
        &vscode_mcp,
        r#"{
            "servers": {
                "unity-vscode": {
                    "url": "http://127.0.0.1:8081/mcp"
                }
            }
        }"#,
    )
    .unwrap();

    // 3. .cursor/mcp.json with "mcpServers"
    let cursor_dir = proj.join(".cursor");
    std::fs::create_dir_all(&cursor_dir).unwrap();
    let cursor_mcp = cursor_dir.join("mcp.json");
    std::fs::write(
        &cursor_mcp,
        r#"{
            "mcpServers": {
                "unity-cursor": {
                    "url": "http://[::1]:8082/mcp"
                }
            }
        }"#,
    )
    .unwrap();

    // Register project in settings
    let mut settings = WorkspaceSettings::default();
    settings
        .project_roots
        .push(proj.to_string_lossy().to_string());
    save_settings_atomic(&cfg, &settings).unwrap();

    let res = discover_mcp_candidates_core(&cfg, false).await.unwrap();

    // Expected 4 candidates: default + 3 from configs
    assert_eq!(res.candidates.len(), 4);
    assert_eq!(res.scanned_roots.len(), 1);
    assert_eq!(res.configs_inspected.len(), 3);

    let default_cand = res
        .candidates
        .iter()
        .find(|c| c.server_name.contains("documented default"))
        .unwrap();
    assert_eq!(
        default_cand.endpoint_url.as_deref(),
        Some(DOCUMENTED_DEFAULT_MCP_ENDPOINT)
    );

    let root_cand = res
        .candidates
        .iter()
        .find(|c| c.server_name == "unity-root")
        .unwrap();
    assert_eq!(
        root_cand.endpoint_url.as_deref(),
        Some("http://localhost:8080/mcp")
    );
    assert_eq!(root_cand.transport, "http");
    assert!(root_cand.supported);

    let vscode_cand = res
        .candidates
        .iter()
        .find(|c| c.server_name == "unity-vscode")
        .unwrap();
    assert_eq!(
        vscode_cand.endpoint_url.as_deref(),
        Some("http://127.0.0.1:8081/mcp")
    );
    assert!(vscode_cand.supported);

    let cursor_cand = res
        .candidates
        .iter()
        .find(|c| c.server_name == "unity-cursor")
        .unwrap();
    assert_eq!(
        cursor_cand.endpoint_url.as_deref(),
        Some("http://[::1]:8082/mcp")
    );
    assert!(cursor_cand.supported);
}

#[tokio::test]
async fn test_unsupported_stdio_transport_safe_handling() {
    let env = TestEnv::new("test_unsupported_stdio");
    let cfg = env.config_dir();
    let proj = env.new_project("stdio_proj");

    let marker_file = proj.join("should_never_exist.txt");
    let escaped_marker = marker_file.to_string_lossy().replace('\\', "\\\\");

    // Config specifying stdio command, args, env with secrets, and auth headers
    let mcp_json = proj.join(".mcp.json");
    std::fs::write(
        &mcp_json,
        format!(
            r#"{{
            "mcpServers": {{
                "unity-stdio-tool": {{
                    "command": "powershell",
                    "args": ["-Command", "New-Item -Path '{}' -ItemType File"],
                    "env": {{
                        "UNITY_LICENSE_KEY": "SUPER_SECRET_KEY_12345",
                        "API_TOKEN": "Bearer xyzSecretToken"
                    }},
                    "headers": {{
                        "Authorization": "Bearer secretHeaderToken"
                    }}
                }}
            }}
        }}"#,
            escaped_marker
        ),
    )
    .unwrap();

    let mut settings = WorkspaceSettings::default();
    settings
        .project_roots
        .push(proj.to_string_lossy().to_string());
    save_settings_atomic(&cfg, &settings).unwrap();

    let res = discover_mcp_candidates_core(&cfg, false).await.unwrap();

    let stdio_cand = res
        .candidates
        .iter()
        .find(|c| c.server_name == "unity-stdio-tool")
        .expect("Stdio candidate should be recorded");

    // Must be marked unsupported
    assert_eq!(stdio_cand.transport, "stdio (unsupported)");
    assert!(!stdio_cand.supported);
    assert!(!stdio_cand.is_approved);
    assert!(stdio_cand.endpoint_url.is_none());

    // Command must NEVER be executed
    assert!(
        !marker_file.exists(),
        "Command must NOT be executed for stdio config"
    );

    // Serialization check: env, headers, tokens, args must NEVER be returned or present
    let serialized = serde_json::to_string(stdio_cand).unwrap();
    assert!(!serialized.contains("SUPER_SECRET_KEY_12345"));
    assert!(!serialized.contains("xyzSecretToken"));
    assert!(!serialized.contains("secretHeaderToken"));
    assert!(!serialized.contains("powershell"));
}

#[tokio::test]
async fn test_oversized_config_file_rejection() {
    let env = TestEnv::new("test_oversized_config");
    let cfg = env.config_dir();
    let proj = env.new_project("oversized_proj");

    // Create a config file larger than 64 KiB
    let mcp_json = proj.join(".mcp.json");
    let mut file = std::fs::File::create(&mcp_json).unwrap();
    let padding = " ".repeat(MAX_MCP_CONFIG_FILE_SIZE as usize + 1024);
    write!(
        file,
        r#"{{"mcpServers": {{"unity": {{"url": "http://localhost:8080/mcp"}}}}}}"#
    )
    .unwrap();
    file.write_all(padding.as_bytes()).unwrap();

    let mut settings = WorkspaceSettings::default();
    settings
        .project_roots
        .push(proj.to_string_lossy().to_string());
    save_settings_atomic(&cfg, &settings).unwrap();

    let res = discover_mcp_candidates_core(&cfg, false).await.unwrap();

    // The oversized file must be rejected and not produce a candidate
    assert!(!res.candidates.iter().any(|c| c.server_name == "unity"));
    assert!(
        res.warnings
            .iter()
            .any(|w| w.contains("exceeds maximum allowed size"))
    );
}

#[tokio::test]
async fn test_grant_persistence_and_revocation() {
    let env = TestEnv::new("test_grant_persistence");
    let cfg = env.config_dir();

    // Initially no grants
    let initial_grants = get_mcp_grants_impl(&cfg).unwrap();
    assert!(initial_grants.is_empty());

    // Initial discovery has default candidate unapproved
    let res = discover_mcp_candidates_core(&cfg, false).await.unwrap();
    let default_cand = res
        .candidates
        .iter()
        .find(|c| c.endpoint_url.as_deref() == Some(DOCUMENTED_DEFAULT_MCP_ENDPOINT))
        .unwrap();
    assert!(!default_cand.is_approved);

    // Approve endpoint with trailing slash to test canonical normalization
    let approve_res = approve_endpoint_impl(&cfg, "http://localhost:8080/mcp/").unwrap();
    assert!(approve_res.is_approved);
    assert_eq!(approve_res.canonical_url, DOCUMENTED_DEFAULT_MCP_ENDPOINT);
    assert_eq!(approve_res.approved_endpoints.len(), 1);

    // Verify persisted on disk and in get_mcp_grants_impl
    let saved_grants = get_mcp_grants_impl(&cfg).unwrap();
    assert_eq!(saved_grants.len(), 1);
    assert_eq!(
        saved_grants[0].canonical_url,
        DOCUMENTED_DEFAULT_MCP_ENDPOINT
    );
    assert!(!saved_grants[0].approved_at.is_empty());

    // Subsequent discovery shows is_approved == true
    let res2 = discover_mcp_candidates_core(&cfg, false).await.unwrap();
    let default_cand2 = res2
        .candidates
        .iter()
        .find(|c| c.endpoint_url.as_deref() == Some(DOCUMENTED_DEFAULT_MCP_ENDPOINT))
        .unwrap();
    assert!(default_cand2.is_approved);

    // Revoke endpoint
    let revoke_res = revoke_endpoint_impl(&cfg, DOCUMENTED_DEFAULT_MCP_ENDPOINT).unwrap();
    assert!(!revoke_res.is_approved);
    assert!(revoke_res.approved_endpoints.is_empty());

    // Verify revoked on disk
    let after_revoke = get_mcp_grants_impl(&cfg).unwrap();
    assert!(after_revoke.is_empty());

    let res3 = discover_mcp_candidates_core(&cfg, false).await.unwrap();
    let default_cand3 = res3
        .candidates
        .iter()
        .find(|c| c.endpoint_url.as_deref() == Some(DOCUMENTED_DEFAULT_MCP_ENDPOINT))
        .unwrap();
    assert!(!default_cand3.is_approved);
}

#[tokio::test]
async fn test_config_cannot_self_approve_and_stale_url_requires_different_grant() {
    let env = TestEnv::new("test_cannot_self_approve");
    let cfg = env.config_dir();
    let proj = env.new_project("game");

    let mcp_json = proj.join(".mcp.json");
    std::fs::write(
        &mcp_json,
        r#"{
            "mcpServers": {
                "unity": {
                    "url": "http://localhost:9090/mcp"
                }
            }
        }"#,
    )
    .unwrap();

    let mut settings = WorkspaceSettings::default();
    settings
        .project_roots
        .push(proj.to_string_lossy().to_string());
    save_settings_atomic(&cfg, &settings).unwrap();

    // 1. Config presence cannot self-approve
    let res = discover_mcp_candidates_core(&cfg, false).await.unwrap();
    let unity_cand = res
        .candidates
        .iter()
        .find(|c| c.server_name == "unity")
        .unwrap();
    assert!(!unity_cand.is_approved);

    // 2. User approves port 9090
    approve_endpoint_impl(&cfg, "http://localhost:9090/mcp").unwrap();
    let res2 = discover_mcp_candidates_core(&cfg, false).await.unwrap();
    assert!(
        res2.candidates
            .iter()
            .find(|c| c.server_name == "unity")
            .unwrap()
            .is_approved
    );

    // 3. Config changes URL to port 9091 (stale/changed URL)
    std::fs::write(
        &mcp_json,
        r#"{
            "mcpServers": {
                "unity": {
                    "url": "http://localhost:9091/mcp"
                }
            }
        }"#,
    )
    .unwrap();

    // Port 9091 must NOT be approved because the grant was for 9090
    let res3 = discover_mcp_candidates_core(&cfg, false).await.unwrap();
    let updated_cand = res3
        .candidates
        .iter()
        .find(|c| c.server_name == "unity")
        .unwrap();
    assert_eq!(
        updated_cand.endpoint_url.as_deref(),
        Some("http://localhost:9091/mcp")
    );
    assert!(
        !updated_cand.is_approved,
        "Changed URL must require a separate approval grant"
    );
}

#[tokio::test]
async fn test_redirect_policy_none_does_not_follow_redirects() {
    // Bind mock HTTP server on random loopback port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    // Spawn server that returns HTTP 302 Found redirect
    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let response = "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:59999/unintended\r\nContent-Length: 0\r\n\r\n";
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let probe_url = format!("http://127.0.0.1:{port}/mcp");
    let result = probe_single_endpoint(&client, &probe_url).await;

    // Must report 302 directly, without following redirect
    match result {
        McpReachability::Reachable { status_code } => {
            assert_eq!(status_code, 302);
        }
        other => panic!("Expected Reachable with status_code 302, got {:?}", other),
    }
}

#[tokio::test]
async fn test_reachability_probe_reporting_status_only_not_identity() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    // Spawn mock server returning 200 OK
    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let n = socket.read(&mut buf).await.unwrap_or(0);
            let req_str = String::from_utf8_lossy(&buf[..n]);
            // Verify reachability probe does NOT send any JSON-RPC initialize payload
            assert!(!req_str.contains("initialize"));
            assert!(!req_str.contains("tools/list"));
            assert!(!req_str.contains("resources/list"));

            let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let probe_url = format!("http://127.0.0.1:{port}/mcp");
    let result = probe_single_endpoint(&client, &probe_url).await;

    match result {
        McpReachability::Reachable { status_code } => {
            assert_eq!(status_code, 200);
        }
        other => panic!("Expected Reachable with status_code 200, got {:?}", other),
    }
}

#[tokio::test]
async fn test_no_autoconnect_and_availability_vs_identity_distinct() {
    let env = TestEnv::new("test_no_autoconnect");
    let cfg = env.config_dir();

    // 1. Without probe: reachability is NotProbed
    let list_res = discover_mcp_candidates_core(&cfg, false).await.unwrap();
    let cand = &list_res.candidates[0];
    assert_eq!(cand.reachability, McpReachability::NotProbed);
    assert_eq!(
        cand.identity_status,
        McpIdentityStatus::Unverified {
            details: "Default candidate is a documented endpoint; server running state and Unity editor identity are unverified".to_string()
        }
    );

    // 2. With probe on unreachable port: reachability is Unreachable, identity remains Unverified
    let disc_res = discover_mcp_candidates_core(&cfg, true).await.unwrap();
    let probed_cand = &disc_res.candidates[0];
    match probed_cand.reachability {
        McpReachability::Unreachable { .. } => {}
        McpReachability::Reachable { .. } => {} // in case something was running on 8080
        _ => panic!("Expected Unreachable or Reachable"),
    }
    match &probed_cand.identity_status {
        McpIdentityStatus::Unverified { details } => {
            assert!(details.contains("unverified"));
        }
    }
}

#[tokio::test]
async fn test_backward_compatibility_old_settings_json() {
    let env = TestEnv::new("test_backward_compat");
    let cfg = env.config_dir();

    // Write older settings.json with NO approvedMcpEndpoints field
    let old_json = r#"{
        "dataRoot": null,
        "projectRoots": [],
        "providerSharingGrants": []
    }"#;
    std::fs::write(cfg.join("settings.json"), old_json).unwrap();

    let grants = get_mcp_grants_impl(&cfg).unwrap();
    assert!(
        grants.is_empty(),
        "Missing field should deserialize to empty vector"
    );

    let res = discover_mcp_candidates_core(&cfg, false).await.unwrap();
    assert!(!res.candidates.is_empty());
    assert!(!res.candidates[0].is_approved);
}

#[tokio::test]
async fn test_invalid_utf8_and_binary_configs_skipped_safely() {
    let env = TestEnv::new("test_invalid_encoding");
    let cfg = env.config_dir();
    let proj = env.new_project("corrupt_proj");

    // Binary file containing null byte
    let mcp_json = proj.join(".mcp.json");
    std::fs::write(&mcp_json, b"{\x00\"mcpServers\": {}}").unwrap();

    let mut settings = WorkspaceSettings::default();
    settings
        .project_roots
        .push(proj.to_string_lossy().to_string());
    save_settings_atomic(&cfg, &settings).unwrap();

    let res = discover_mcp_candidates_core(&cfg, false).await.unwrap();
    assert!(
        res.warnings
            .iter()
            .any(|w| w.contains("contains binary data"))
    );
}
