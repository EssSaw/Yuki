use std::io::Write;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::client::{
    bind_unity_instance_core, connect_approved_mcp_endpoint_core, disconnect_session_core,
    get_bound_unity_state_core, list_connected_unity_instances_core, parse_unity_instances,
    send_jsonrpc_post, verify_project_root_locally,
};
use super::discovery::{discover_mcp_candidates_core, probe_single_endpoint};
use super::grants::{approve_endpoint_impl, get_mcp_grants_impl, revoke_endpoint_impl};
use super::models::{
    DOCUMENTED_DEFAULT_MCP_ENDPOINT, MAX_ACTIVE_MCP_SESSIONS, MAX_MCP_CONFIG_FILE_SIZE,
    MAX_MCP_RESPONSE_BYTES, McpIdentityStatus, McpReachability, UnityProjectVerificationStatus,
};
use super::protocol::{SseEvent, extract_jsonrpc_from_sse, parse_sse_frames};
use super::session::{McpSession, McpSessionManager};
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

async fn read_mock_http_request(socket: &mut tokio::net::TcpStream) -> (String, String) {
    let mut data = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = match socket.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        data.extend_from_slice(&buf[..n]);
        if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers_str = String::from_utf8_lossy(&data[..pos]);
            let content_len = headers_str
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if data.len() >= pos + 4 + content_len {
                break;
            }
        }
    }
    let req_str = String::from_utf8_lossy(&data).to_string();
    let header_end = req_str.find("\r\n\r\n").unwrap_or(req_str.len());
    let headers = req_str[..header_end].to_string();
    let body = if header_end + 4 < req_str.len() {
        req_str[header_end + 4..].to_string()
    } else {
        String::new()
    };
    (headers, body)
}

async fn send_mock_http_response(
    socket: &mut tokio::net::TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
    extra_headers: &[(&str, &str)],
) {
    let mut resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in extra_headers {
        resp.push_str(&format!("{k}: {v}\r\n"));
    }
    resp.push_str("\r\n");
    resp.push_str(body);
    let _ = socket.write_all(resp.as_bytes()).await;
}

#[tokio::test]
async fn test_protocol_unsupported_version() {
    let env = TestEnv::new("test_unsupported_ver");
    let cfg = env.config_dir();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/mcp");

    approve_endpoint_impl(&cfg, &url).unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let (_, _) = read_mock_http_request(&mut socket).await;
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "protocolVersion": "99.0.0"
                }
            });
            send_mock_http_response(
                &mut socket,
                "200 OK",
                "application/json",
                &resp.to_string(),
                &[],
            )
            .await;
        }
    });

    let session_mgr = McpSessionManager::new();
    let res = connect_approved_mcp_endpoint_core(&cfg, &session_mgr, &url).await;
    assert!(res.is_err());
    let err = res.err().unwrap();
    assert!(err.contains("Unsupported MCP protocol version '99.0.0'"));
}

#[tokio::test]
async fn test_protocol_invalid_jsonrpc_missing_field() {
    let env = TestEnv::new("test_invalid_jsonrpc");
    let cfg = env.config_dir();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/mcp");

    approve_endpoint_impl(&cfg, &url).unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let (_, _) = read_mock_http_request(&mut socket).await;
            let resp = serde_json::json!({
                "id": 1,
                "result": {
                    "protocolVersion": "2024-11-05"
                }
            });
            send_mock_http_response(
                &mut socket,
                "200 OK",
                "application/json",
                &resp.to_string(),
                &[],
            )
            .await;
        }
    });

    let session_mgr = McpSessionManager::new();
    let res = connect_approved_mcp_endpoint_core(&cfg, &session_mgr, &url).await;
    assert!(res.is_err());
    assert!(res.err().unwrap().contains("Invalid JSON-RPC protocol"));
}

#[tokio::test]
async fn test_protocol_response_id_mismatch() {
    let env = TestEnv::new("test_id_mismatch");
    let cfg = env.config_dir();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/mcp");

    approve_endpoint_impl(&cfg, &url).unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let (_, _) = read_mock_http_request(&mut socket).await;
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 999,
                "result": {
                    "protocolVersion": "2024-11-05"
                }
            });
            send_mock_http_response(
                &mut socket,
                "200 OK",
                "application/json",
                &resp.to_string(),
                &[],
            )
            .await;
        }
    });

    let session_mgr = McpSessionManager::new();
    let res = connect_approved_mcp_endpoint_core(&cfg, &session_mgr, &url).await;
    assert!(res.is_err());
    assert!(
        res.err()
            .unwrap()
            .contains("Mismatched JSON-RPC response ID")
    );
}

#[tokio::test]
async fn test_protocol_jsonrpc_error_response() {
    let env = TestEnv::new("test_jsonrpc_error");
    let cfg = env.config_dir();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/mcp");

    approve_endpoint_impl(&cfg, &url).unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let (_, _) = read_mock_http_request(&mut socket).await;
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {
                    "code": -32601,
                    "message": "Method not found"
                }
            });
            send_mock_http_response(
                &mut socket,
                "200 OK",
                "application/json",
                &resp.to_string(),
                &[],
            )
            .await;
        }
    });

    let session_mgr = McpSessionManager::new();
    let res = connect_approved_mcp_endpoint_core(&cfg, &session_mgr, &url).await;
    assert!(res.is_err());
    assert!(
        res.err()
            .unwrap()
            .contains("MCP error -32601: Method not found")
    );
}

#[tokio::test]
async fn test_protocol_malformed_json_response() {
    let env = TestEnv::new("test_malformed_json");
    let cfg = env.config_dir();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/mcp");

    approve_endpoint_impl(&cfg, &url).unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let (_, _) = read_mock_http_request(&mut socket).await;
            send_mock_http_response(
                &mut socket,
                "200 OK",
                "application/json",
                "not valid json",
                &[],
            )
            .await;
        }
    });

    let session_mgr = McpSessionManager::new();
    let res = connect_approved_mcp_endpoint_core(&cfg, &session_mgr, &url).await;
    assert!(res.is_err());
    assert!(res.err().unwrap().contains("Invalid JSON in MCP response"));
}

#[test]
fn test_sse_parsing_frames_and_multiline_data() {
    let raw_sse = "event: message\r\ndata: line 1\r\ndata: line 2\r\n: this is a comment\r\nid: evt-1\r\n\r\nevent: ping\r\ndata: pong\r\n\r\n";
    let (events, consumed) = parse_sse_frames(raw_sse);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type.as_deref(), Some("message"));
    assert_eq!(events[0].data, "line 1\nline 2");
    assert_eq!(events[0].id.as_deref(), Some("evt-1"));
    assert_eq!(events[1].event_type.as_deref(), Some("ping"));
    assert_eq!(events[1].data, "pong");
    assert_eq!(consumed, raw_sse.len());

    // Incomplete frame (missing trailing delimiter)
    let partial = "event: message\ndata: {\"jsonrpc\":\"2.0\"}\n";
    let (ev_partial, consumed_partial) = parse_sse_frames(partial);
    assert_eq!(ev_partial.len(), 0);
    assert_eq!(consumed_partial, 0);

    // Extract JSON-RPC
    let json_event = SseEvent {
        event_type: Some("message".to_string()),
        data: r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#.to_string(),
        id: None,
    };
    let parsed = extract_jsonrpc_from_sse(&[json_event], Some(1)).unwrap();
    assert_eq!(parsed.get("jsonrpc").and_then(|v| v.as_str()), Some("2.0"));
    assert_eq!(parsed.get("id").and_then(|v| v.as_u64()), Some(1));
}

#[tokio::test]
async fn test_connect_and_negotiate_via_sse() {
    let env = TestEnv::new("test_sse_connect");
    let cfg = env.config_dir();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/mcp");

    approve_endpoint_impl(&cfg, &url).unwrap();

    tokio::spawn(async move {
        // Request 1: initialize
        if let Ok((mut socket, _)) = listener.accept().await {
            let (headers, body) = read_mock_http_request(&mut socket).await;
            assert!(headers.contains("mcp-protocol-version: 2024-11-05"));
            assert!(body.contains("initialize"));

            let sse_body = "event: message\r\ndata: {\"jsonrpc\": \"2.0\", \"id\": 1, \"result\": {\"protocolVersion\": \"2024-11-05\", \"serverInfo\": {\"name\": \"MockUnitySse\", \"version\": \"1.0.0\"}}}\r\n\r\n";
            send_mock_http_response(
                &mut socket,
                "200 OK",
                "text/event-stream",
                sse_body,
                &[("mcp-session-id", "mock-session-sse-999")],
            )
            .await;
        }

        // Request 2: notifications/initialized
        if let Ok((mut socket, _)) = listener.accept().await {
            let (headers, body) = read_mock_http_request(&mut socket).await;
            assert!(headers.contains("mcp-session-id: mock-session-sse-999"));
            assert!(body.contains("notifications/initialized"));
            send_mock_http_response(&mut socket, "200 OK", "application/json", "", &[]).await;
        }
    });

    let session_mgr = McpSessionManager::new();
    let conn = connect_approved_mcp_endpoint_core(&cfg, &session_mgr, &url)
        .await
        .unwrap();

    assert_eq!(conn.protocol_version, "2024-11-05");
    assert_eq!(conn.server_name.as_deref(), Some("MockUnitySse"));
    assert_eq!(conn.server_version.as_deref(), Some("1.0.0"));

    session_mgr
        .with_session(&conn.session_id, |s| {
            assert_eq!(s.mcp_session_id.as_deref(), Some("mock-session-sse-999"));
            assert_eq!(s.protocol_version, "2024-11-05");
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_connect_unapproved_endpoint_denied() {
    let env = TestEnv::new("test_unapproved_connect");
    let cfg = env.config_dir();
    let session_mgr = McpSessionManager::new();

    let res =
        connect_approved_mcp_endpoint_core(&cfg, &session_mgr, "http://localhost:8080/mcp").await;
    assert!(res.is_err());
    let err = res.err().unwrap();
    assert!(err.contains("is not approved or approval grant has been revoked"));
}

#[tokio::test]
async fn test_operations_on_unapproved_endpoint_denied() {
    let env = TestEnv::new("test_unapproved_ops");
    let cfg = env.config_dir();
    let session_mgr = McpSessionManager::new();

    // Create a session in memory for an endpoint that is NOT approved in settings
    let session = McpSession::new(
        "test-sess-1".to_string(),
        "http://localhost:8080/mcp".to_string(),
        "2024-11-05".to_string(),
        None,
        None,
        None,
    );
    session_mgr.add_session(session).await.unwrap();

    // 1. list_connected_unity_instances_core must reject and invalidate
    let list_res = list_connected_unity_instances_core(&cfg, &session_mgr, "test-sess-1").await;
    assert!(list_res.is_err());
    assert!(list_res.err().unwrap().contains("is not approved"));
    assert_eq!(session_mgr.active_count().await, 0);

    // 2. Re-add and test bind
    let session2 = McpSession::new(
        "test-sess-2".to_string(),
        "http://localhost:8080/mcp".to_string(),
        "2024-11-05".to_string(),
        None,
        None,
        None,
    );
    session_mgr.add_session(session2).await.unwrap();
    let bind_res = bind_unity_instance_core(&cfg, &session_mgr, "test-sess-2", "Game@abc123").await;
    assert!(bind_res.is_err());
    assert!(bind_res.err().unwrap().contains("is not approved"));
    assert_eq!(session_mgr.active_count().await, 0);

    // 3. Re-add and test get_bound_unity_state
    let mut session3 = McpSession::new(
        "test-sess-3".to_string(),
        "http://localhost:8080/mcp".to_string(),
        "2024-11-05".to_string(),
        None,
        None,
        None,
    );
    session3.bound_instance_id = Some("Game@abc123".to_string());
    session_mgr.add_session(session3).await.unwrap();
    let state_res = get_bound_unity_state_core(&cfg, &session_mgr, "test-sess-3").await;
    assert!(state_res.is_err());
    assert!(state_res.err().unwrap().contains("is not approved"));
    assert_eq!(session_mgr.active_count().await, 0);
}

#[test]
fn test_verify_project_root_locally() {
    let env = TestEnv::new("test_verify_root");
    let cfg = env.config_dir();
    let proj = env.new_project("MyUnityGame");
    let unapproved = env.new_project("OtherGame");

    let mut settings = WorkspaceSettings::default();
    settings
        .project_roots
        .push(proj.to_string_lossy().to_string());
    settings
        .provider_sharing_grants
        .push(proj.to_string_lossy().to_string());
    save_settings_atomic(&cfg, &settings).unwrap();

    // 1. Exact match with sharing grant enabled
    let (v1, s1) = verify_project_root_locally(&cfg, Some(&proj.to_string_lossy())).unwrap();
    assert!(matches!(
        v1,
        UnityProjectVerificationStatus::Verified {
            provider_sharing_enabled: true,
            ..
        }
    ));
    assert!(s1);

    // 2. Exact match with sharing grant disabled
    settings.provider_sharing_grants.clear();
    save_settings_atomic(&cfg, &settings).unwrap();
    let (v2, s2) = verify_project_root_locally(&cfg, Some(&proj.to_string_lossy())).unwrap();
    assert!(matches!(
        v2,
        UnityProjectVerificationStatus::Verified {
            provider_sharing_enabled: false,
            ..
        }
    ));
    assert!(!s2);

    // 3. Existing folder that is NOT approved in project_roots
    let (v3, s3) = verify_project_root_locally(&cfg, Some(&unapproved.to_string_lossy())).unwrap();
    assert!(matches!(
        v3,
        UnityProjectVerificationStatus::Mismatched { .. }
    ));
    assert!(!s3);

    // 4. None reported (missing projectRoot)
    let (v4, s4) = verify_project_root_locally(&cfg, None).unwrap();
    match v4 {
        UnityProjectVerificationStatus::Unverifiable { reason } => {
            assert!(reason.contains("Project root cannot be inferred from instance name alone"));
        }
        _ => panic!("Expected Unverifiable"),
    }
    assert!(!s4);

    // 5. Non-existent path on disk
    let non_existent = dunce::canonicalize(std::env::current_dir().unwrap())
        .unwrap()
        .join("target")
        .join("does_not_exist_folder_9999");
    let (v5, s5) =
        verify_project_root_locally(&cfg, Some(&non_existent.to_string_lossy())).unwrap();
    assert!(matches!(
        v5,
        UnityProjectVerificationStatus::Unverifiable { .. }
    ));
    assert!(!s5);
}

#[tokio::test]
async fn test_bind_unity_instance_success_and_state() {
    let env = TestEnv::new("test_bind_success");
    let cfg = env.config_dir();
    let proj = env.new_project("UnityGame");

    let mut settings = WorkspaceSettings::default();
    settings
        .project_roots
        .push(proj.to_string_lossy().to_string());
    settings
        .provider_sharing_grants
        .push(proj.to_string_lossy().to_string());
    save_settings_atomic(&cfg, &settings).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/mcp");
    approve_endpoint_impl(&cfg, &url).unwrap();

    let proj_path_str = proj.to_string_lossy().to_string();

    tokio::spawn(async move {
        // Step 1: tools/call set_active_instance
        if let Ok((mut socket, _)) = listener.accept().await {
            let (headers, body) = read_mock_http_request(&mut socket).await;
            assert!(body.contains("set_active_instance"));
            assert!(headers.contains("mcp-protocol-version: 2024-11-05"));
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "isError": false,
                    "content": [{"type": "text", "text": "Switched to UnityGame@abc123"}]
                }
            });
            send_mock_http_response(
                &mut socket,
                "200 OK",
                "application/json",
                &resp.to_string(),
                &[],
            )
            .await;
        }

        // Step 2: resources/read mcpforunity://editor/state
        if let Ok((mut socket, _)) = listener.accept().await {
            let (headers, body) = read_mock_http_request(&mut socket).await;
            assert!(body.contains("editor/state"));
            assert!(headers.contains("mcp-protocol-version: 2024-11-05"));
            let editor_state_payload = serde_json::json!({
                "isPlaying": false,
                "isPaused": false,
                "isCompiling": false,
                "unityVersion": "6000.0.23f1",
                "projectRoot": proj_path_str
            });
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "result": {
                    "contents": [{
                        "uri": "mcpforunity://editor/state",
                        "text": editor_state_payload.to_string()
                    }]
                }
            });
            send_mock_http_response(
                &mut socket,
                "200 OK",
                "application/json",
                &resp.to_string(),
                &[],
            )
            .await;
        }
    });

    let session_mgr = McpSessionManager::new();
    let session = McpSession::new(
        "sess-bind-1".to_string(),
        url.clone(),
        "2024-11-05".to_string(),
        None,
        None,
        None,
    );
    session_mgr.add_session(session).await.unwrap();

    let bound_state =
        bind_unity_instance_core(&cfg, &session_mgr, "sess-bind-1", "UnityGame@abc123")
            .await
            .unwrap();

    assert_eq!(bound_state.bound_instance_id, "UnityGame@abc123");
    assert_eq!(bound_state.unity_version.as_deref(), Some("6000.0.23f1"));
    assert!(!bound_state.is_playing);
    assert!(!bound_state.is_compiling);
    assert!(bound_state.provider_sharing_eligible);
    assert!(matches!(
        bound_state.project_verification,
        UnityProjectVerificationStatus::Verified { .. }
    ));
}

#[tokio::test]
async fn test_bind_unity_instance_invalid_id_format() {
    let env = TestEnv::new("test_invalid_id");
    let cfg = env.config_dir();
    let session_mgr = McpSessionManager::new();

    let session = McpSession::new(
        "sess-inv".to_string(),
        "http://localhost:8080/mcp".to_string(),
        "2024-11-05".to_string(),
        None,
        None,
        None,
    );
    session_mgr.add_session(session).await.unwrap();

    let res = bind_unity_instance_core(&cfg, &session_mgr, "sess-inv", "JustNameWithoutHash").await;
    assert!(res.is_err());
    assert!(res.err().unwrap().contains("expected exact 'Name@hash'"));
}

#[tokio::test]
async fn test_revocation_invalidates_active_sessions() {
    let env = TestEnv::new("test_revocation_inv");
    let cfg = env.config_dir();
    let session_mgr = McpSessionManager::new();
    let endpoint = "http://127.0.0.1:8080/mcp";

    approve_endpoint_impl(&cfg, endpoint).unwrap();

    let session = McpSession::new(
        "sess-rev".to_string(),
        endpoint.to_string(),
        "2024-11-05".to_string(),
        None,
        None,
        None,
    );
    session_mgr.add_session(session).await.unwrap();
    assert_eq!(session_mgr.active_count().await, 1);

    // Invalidate endpoint sessions (as performed by revoke_mcp_endpoint command)
    session_mgr.invalidate_endpoint_sessions(endpoint).await;
    assert_eq!(session_mgr.active_count().await, 0);

    let res = session_mgr.with_session("sess-rev", |_| Ok(())).await;
    assert!(res.is_err());

    // Revoke endpoint grant from settings
    revoke_endpoint_impl(&cfg, endpoint).unwrap();
    let grants = get_mcp_grants_impl(&cfg).unwrap();
    assert!(grants.is_empty());
}

#[tokio::test]
async fn test_session_404_invalidates_session_without_replay() {
    let env = TestEnv::new("test_session_404");
    let cfg = env.config_dir();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/mcp");
    approve_endpoint_impl(&cfg, &url).unwrap();

    let request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count_clone = request_count.clone();

    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (_, _) = read_mock_http_request(&mut socket).await;
            send_mock_http_response(
                &mut socket,
                "404 Not Found",
                "text/plain",
                "Session Not Found",
                &[],
            )
            .await;
        }
    });

    let session_mgr = McpSessionManager::new();
    let session = McpSession::new(
        "sess-404".to_string(),
        url.clone(),
        "2024-11-05".to_string(),
        Some("old-session-token".to_string()),
        None,
        None,
    );
    session_mgr.add_session(session).await.unwrap();

    let res = list_connected_unity_instances_core(&cfg, &session_mgr, "sess-404").await;
    assert!(res.is_err());
    let err = res.err().unwrap();
    assert!(err.contains("SESSION_404:"));

    // Server must have received exactly 1 request (no blind retry or replay)
    assert_eq!(request_count.load(std::sync::atomic::Ordering::SeqCst), 1);

    // Session must be removed from manager
    assert_eq!(session_mgr.active_count().await, 0);
    assert!(
        session_mgr
            .with_session("sess-404", |_| Ok(()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn test_bounded_response_size_limit_plain_json() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/mcp");

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let (_, _) = read_mock_http_request(&mut socket).await;
            // Send body larger than 1 MiB (MAX_MCP_RESPONSE_BYTES)
            let padding = "x".repeat(MAX_MCP_RESPONSE_BYTES + 1024);
            let body = format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"{padding}\"}}");
            send_mock_http_response(&mut socket, "200 OK", "application/json", &body, &[]).await;
        }
    });

    let client = reqwest::Client::builder().build().unwrap();
    let req = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "test"});
    let res = send_jsonrpc_post(&client, &url, None, None, &req, Some(1)).await;

    assert!(res.is_err());
    let err = res.err().unwrap();
    assert!(err.contains("size exceeded limit of 1048576 bytes"));
}

#[tokio::test]
async fn test_bounded_response_size_limit_sse() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/mcp");

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let (_, _) = read_mock_http_request(&mut socket).await;
            let padding = "x".repeat(MAX_MCP_RESPONSE_BYTES + 1024);
            let sse_body = format!("event: message\r\ndata: {padding}\r\n\r\n");
            send_mock_http_response(&mut socket, "200 OK", "text/event-stream", &sse_body, &[])
                .await;
        }
    });

    let client = reqwest::Client::builder().build().unwrap();
    let req = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "test"});
    let res = send_jsonrpc_post(&client, &url, None, None, &req, Some(1)).await;

    assert!(res.is_err());
    let err = res.err().unwrap();
    assert!(err.contains("size exceeded limit of 1048576 bytes"));
}

#[tokio::test]
async fn test_bounded_active_sessions_limit_and_disconnect() {
    let session_mgr = McpSessionManager::new();

    // Fill up to MAX_ACTIVE_MCP_SESSIONS (5)
    for i in 1..=MAX_ACTIVE_MCP_SESSIONS {
        let session = McpSession::new(
            format!("sess-{i}"),
            "http://localhost:8080/mcp".to_string(),
            "2024-11-05".to_string(),
            None,
            None,
            None,
        );
        session_mgr.add_session(session).await.unwrap();
    }
    assert_eq!(session_mgr.active_count().await, 5);

    // 6th session must be rejected
    let overflow_session = McpSession::new(
        "sess-overflow".to_string(),
        "http://localhost:8080/mcp".to_string(),
        "2024-11-05".to_string(),
        None,
        None,
        None,
    );
    let res = session_mgr.add_session(overflow_session).await;
    assert!(res.is_err());
    assert!(
        res.err()
            .unwrap()
            .contains("Maximum active MCP sessions limit (5) reached")
    );

    // Disconnect one session
    let disconn = disconnect_session_core(&session_mgr, "sess-1").await;
    assert!(disconn.success);
    assert_eq!(session_mgr.active_count().await, 4);

    // Now adding succeeds
    let ok_session = McpSession::new(
        "sess-ok".to_string(),
        "http://localhost:8080/mcp".to_string(),
        "2024-11-05".to_string(),
        None,
        None,
        None,
    );
    session_mgr.add_session(ok_session).await.unwrap();
    assert_eq!(session_mgr.active_count().await, 5);
}

#[test]
fn test_parse_unity_instances_formats() {
    // Array of strings
    let raw_strings = serde_json::json!(["ProjectA@111", "ProjectB@222"]);
    let parsed1 = parse_unity_instances(&raw_strings).unwrap();
    assert_eq!(parsed1.len(), 2);
    assert_eq!(parsed1[0].name, "ProjectA");
    assert_eq!(parsed1[0].hash, "111");

    // Array of objects with id Name@hash
    let raw_objs = serde_json::json!([
        {"id": "ProjectC@333", "is_active": true, "port": 8080}
    ]);
    let parsed2 = parse_unity_instances(&raw_objs).unwrap();
    assert_eq!(parsed2.len(), 1);
    assert_eq!(parsed2[0].name, "ProjectC");
    assert_eq!(parsed2[0].hash, "333");
    assert!(parsed2[0].is_active);
    assert_eq!(parsed2[0].port, Some(8080));

    // Object with instances key
    let raw_wrapped = serde_json::json!({
        "instances": [{"name": "ProjectD", "hash": "444"}]
    });
    let parsed3 = parse_unity_instances(&raw_wrapped).unwrap();
    assert_eq!(parsed3.len(), 1);
    assert_eq!(parsed3[0].id, "ProjectD@444");

    // Invalid format (no hash)
    let raw_invalid = serde_json::json!(["InvalidPlainName"]);
    assert!(parse_unity_instances(&raw_invalid).is_err());
}
