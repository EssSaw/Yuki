use futures_util::StreamExt;
use std::path::Path;
use std::time::Duration;

use crate::workspace::settings::load_settings;

use super::models::{
    BoundUnityState, DEFAULT_MCP_PROTOCOL_VERSION, MAX_MCP_RESPONSE_BYTES, MCP_REQUEST_TIMEOUT_MS,
    McpConnectionResult, McpDisconnectResult, UnityInstanceInfo, UnityProjectVerificationStatus,
};
use super::protocol::{
    build_initialize_request, build_initialized_notification, build_resources_read_request,
    build_set_active_instance_request, extract_jsonrpc_from_sse, is_supported_mcp_version,
    parse_sse_frames,
};
use super::session::{McpSession, McpSessionManager};
use super::url::validate_and_canonicalize_loopback_url;

/// Checks that an endpoint URL is currently approved in workspace settings.
pub fn ensure_endpoint_is_approved(config_dir: &Path, canonical_url: &str) -> Result<(), String> {
    let settings = load_settings(config_dir)?;
    let is_approved = settings
        .approved_mcp_endpoints
        .iter()
        .any(|g| g.canonical_url == canonical_url);
    if !is_approved {
        return Err(format!(
            "MCP endpoint '{canonical_url}' is not approved or approval grant has been revoked."
        ));
    }
    Ok(())
}

/// Builds an HTTP client with redirects strictly disabled and bounded timeouts.
pub fn build_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(MCP_REQUEST_TIMEOUT_MS))
        .connect_timeout(Duration::from_millis(MCP_REQUEST_TIMEOUT_MS))
        .build()
        .map_err(|e| format!("Failed to create MCP HTTP client: {e}"))
}

/// Sends a JSON-RPC request via HTTP POST, handling JSON or SSE responses and strict validation.
pub async fn send_jsonrpc_post(
    client: &reqwest::Client,
    endpoint: &str,
    session_header: Option<&str>,
    protocol_version: Option<&str>,
    body: &serde_json::Value,
    expected_id: Option<u64>,
) -> Result<(serde_json::Value, Option<String>), String> {
    let mut req = client
        .post(endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(body);

    if let Some(hdr) = session_header {
        req = req.header("Mcp-Session-Id", hdr);
    }
    if let Some(pv) = protocol_version {
        req = req.header("Mcp-Protocol-Version", pv);
    }

    let resp = req.send().await.map_err(|e| {
        if e.is_timeout() {
            format!("MCP request timed out after {MCP_REQUEST_TIMEOUT_MS}ms")
        } else if e.is_connect() {
            "Connection refused (no MCP server listening on endpoint)".to_string()
        } else {
            format!("HTTP request failed: {e}")
        }
    })?;

    // Handle 404: distinguish initial endpoint missing from expired established session
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        if session_header.is_some() {
            return Err(
                "SESSION_404: MCP server returned 404 Not Found (session expired or not found). A fresh connection and explicit rebind is required without replay."
                    .to_string(),
            );
        } else {
            return Err(
                "ENDPOINT_404: MCP server returned 404 Not Found (endpoint URL not found on server). Check the server configuration or URL path."
                    .to_string(),
            );
        }
    }

    if !resp.status().is_success() {
        return Err(format!(
            "MCP server returned HTTP error status {}",
            resp.status()
        ));
    }

    // Capture Mcp-Session-Id response header if returned by server
    let new_session_hdr = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let is_sse = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("text/event-stream"))
        .unwrap_or(false);

    let json_val = if is_sse {
        read_sse_jsonrpc_response(resp, expected_id).await?
    } else {
        read_plain_jsonrpc_response(resp, expected_id).await?
    };

    // Strict JSON-RPC 2.0 response validation
    if json_val.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
        return Err(
            "Invalid JSON-RPC protocol: missing or incorrect 'jsonrpc': '2.0' field in response"
                .to_string(),
        );
    }

    // Check error before ID check so error responses are never masked
    if let Some(err_obj) = json_val.get("error") {
        let code = err_obj.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        let message = err_obj
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown JSON-RPC error");
        return Err(format!("MCP error {code}: {message}"));
    }

    if let Some(exp_id) = expected_id {
        let resp_id = json_val.get("id").and_then(|v| v.as_u64());
        if resp_id != Some(exp_id) {
            return Err(format!(
                "Mismatched JSON-RPC response ID: expected {exp_id}, got {resp_id:?}"
            ));
        }
    }

    Ok((json_val, new_session_hdr))
}

async fn read_sse_jsonrpc_response(
    resp: reqwest::Response,
    expected_id: Option<u64>,
) -> Result<serde_json::Value, String> {
    let mut stream = resp.bytes_stream();
    let mut raw_buffer = Vec::new();
    let mut total_bytes = 0usize;

    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|e| format!("Error streaming SSE chunk: {e}"))?;
        total_bytes += chunk.len();
        if total_bytes > MAX_MCP_RESPONSE_BYTES {
            return Err(format!(
                "MCP response size exceeded limit of {MAX_MCP_RESPONSE_BYTES} bytes"
            ));
        }

        raw_buffer.extend_from_slice(&chunk);

        let valid_len = match std::str::from_utf8(&raw_buffer) {
            Ok(_) => raw_buffer.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(e) => return Err(format!("Invalid UTF-8 in SSE stream: {e}")),
        };

        if valid_len > 0 {
            let valid_str = match std::str::from_utf8(&raw_buffer[..valid_len]) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let (events, consumed) = parse_sse_frames(valid_str);
            if consumed > 0 {
                raw_buffer.drain(..consumed);
            }

            if let Some(val) = extract_jsonrpc_from_sse(&events, expected_id) {
                return Ok(val);
            }
        }
    }

    // Process any remaining buffer at end of stream
    if !raw_buffer.is_empty() {
        let rem_str = match std::str::from_utf8(&raw_buffer) {
            Ok(s) => s,
            Err(e) => return Err(format!("Invalid UTF-8 at end of SSE stream: {e}")),
        };
        let (events, _) = parse_sse_frames(rem_str);
        if let Some(val) = extract_jsonrpc_from_sse(&events, expected_id) {
            return Ok(val);
        }
        // Fallback: server may have returned plain JSON body under text/event-stream header
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(rem_str.trim()) {
            if val.is_object() && val.get("jsonrpc").and_then(|v| v.as_str()) == Some("2.0") {
                return Ok(val);
            }
        }
    }

    Err("No matching JSON-RPC response found in SSE stream".to_string())
}

async fn read_plain_jsonrpc_response(
    resp: reqwest::Response,
    _expected_id: Option<u64>,
) -> Result<serde_json::Value, String> {
    let mut stream = resp.bytes_stream();
    let mut body_bytes = Vec::new();

    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|e| format!("Error reading response body: {e}"))?;
        if body_bytes.len() + chunk.len() > MAX_MCP_RESPONSE_BYTES {
            return Err(format!(
                "MCP response size exceeded limit of {MAX_MCP_RESPONSE_BYTES} bytes"
            ));
        }
        body_bytes.extend_from_slice(&chunk);
    }

    let json_val: serde_json::Value = serde_json::from_slice(&body_bytes)
        .map_err(|e| format!("Invalid JSON in MCP response: {e}"))?;
    Ok(json_val)
}

/// Sends a JSON-RPC notification (no expected response body/ID).
pub async fn send_jsonrpc_notification(
    client: &reqwest::Client,
    endpoint: &str,
    session_header: Option<&str>,
    protocol_version: Option<&str>,
    body: &serde_json::Value,
) -> Result<Option<String>, String> {
    let mut req = client
        .post(endpoint)
        .header("Content-Type", "application/json")
        .json(body);

    if let Some(hdr) = session_header {
        req = req.header("Mcp-Session-Id", hdr);
    }
    if let Some(pv) = protocol_version {
        req = req.header("Mcp-Protocol-Version", pv);
    }

    let resp = req.send().await.map_err(|e| {
        if e.is_timeout() {
            format!("MCP notification timed out after {MCP_REQUEST_TIMEOUT_MS}ms")
        } else {
            format!("HTTP notification failed: {e}")
        }
    })?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        if session_header.is_some() {
            return Err("SESSION_404: MCP server returned 404 Not Found on notification (session expired or not found).".to_string());
        } else {
            return Err("ENDPOINT_404: MCP server returned 404 Not Found on notification (endpoint URL not found on server).".to_string());
        }
    }

    let new_session_hdr = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    Ok(new_session_hdr)
}

/// Connects to an approved loopback MCP endpoint, negotiates protocol version,
/// sends initialized notification, and registers the active in-memory session.
pub async fn connect_approved_mcp_endpoint_core(
    config_dir: &Path,
    session_manager: &McpSessionManager,
    endpoint: &str,
) -> Result<McpConnectionResult, String> {
    let canonical = validate_and_canonicalize_loopback_url(endpoint)?;
    ensure_endpoint_is_approved(config_dir, &canonical)?;

    let client = build_http_client()?;

    // Step 1: Send initialize request
    let init_req = build_initialize_request(1, DEFAULT_MCP_PROTOCOL_VERSION);
    let (init_resp, session_hdr) = send_jsonrpc_post(
        &client,
        &canonical,
        None,
        Some(DEFAULT_MCP_PROTOCOL_VERSION),
        &init_req,
        Some(1),
    )
    .await?;

    let result = init_resp
        .get("result")
        .ok_or_else(|| "MCP initialize response missing 'result' object".to_string())?;

    let server_proto_ver = result
        .get("protocolVersion")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_MCP_PROTOCOL_VERSION);

    if !is_supported_mcp_version(server_proto_ver) {
        return Err(format!(
            "Unsupported MCP protocol version '{server_proto_ver}'. Supported versions: {}",
            super::models::SUPPORTED_MCP_PROTOCOL_VERSIONS.join(", ")
        ));
    }

    let server_info = result.get("serverInfo");
    let server_name = server_info
        .and_then(|si| si.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let server_version = server_info
        .and_then(|si| si.get("version"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Step 2: Send notifications/initialized
    let init_notif = build_initialized_notification();
    let notif_session_hdr = send_jsonrpc_notification(
        &client,
        &canonical,
        session_hdr.as_deref(),
        Some(server_proto_ver),
        &init_notif,
    )
    .await?;

    let effective_session_hdr = notif_session_hdr.or(session_hdr);

    // Step 3: Register native session in manager
    let session_uuid = uuid::Uuid::new_v4().to_string();
    let session = McpSession::new(
        session_uuid.clone(),
        canonical.clone(),
        server_proto_ver.to_string(),
        effective_session_hdr,
        server_name.clone(),
        server_version.clone(),
    );

    session_manager.add_session(session).await?;

    Ok(McpConnectionResult {
        session_id: session_uuid,
        endpoint_url: canonical,
        protocol_version: server_proto_ver.to_string(),
        server_name,
        server_version,
    })
}

/// Queries the connected Unity instances resource (`mcpforunity://instances`) via MCP.
/// The raw inventory is returned strictly to the local UI; it is never forwarded to AI models.
pub async fn list_connected_unity_instances_core(
    config_dir: &Path,
    session_manager: &McpSessionManager,
    session_id: &str,
) -> Result<Vec<UnityInstanceInfo>, String> {
    let (canonical_endpoint, session_hdr, protocol_version, req_id) = session_manager
        .with_session(session_id, |s| {
            Ok((
                s.canonical_endpoint.clone(),
                s.mcp_session_id.clone(),
                s.protocol_version.clone(),
                s.next_request_id(),
            ))
        })
        .await?;

    // Check approval grant
    if let Err(e) = ensure_endpoint_is_approved(config_dir, &canonical_endpoint) {
        session_manager.invalidate_session(session_id).await;
        return Err(e);
    }

    let client = build_http_client()?;
    let req_body = build_resources_read_request(req_id, "mcpforunity://instances");

    let (resp_json, new_hdr) = match send_jsonrpc_post(
        &client,
        &canonical_endpoint,
        session_hdr.as_deref(),
        Some(&protocol_version),
        &req_body,
        Some(req_id),
    )
    .await
    {
        Ok(res) => res,
        Err(e) => {
            if e.contains("SESSION_404") {
                session_manager.invalidate_session(session_id).await;
            }
            return Err(e);
        }
    };

    if let Some(h) = new_hdr {
        let _ = session_manager
            .with_session_mut(session_id, |s| {
                s.mcp_session_id = Some(h);
                Ok(())
            })
            .await;
    }

    let contents = resp_json
        .get("result")
        .and_then(|r| r.get("contents"))
        .and_then(|c| c.as_array())
        .ok_or_else(|| {
            "Invalid MCP resources/read response: missing 'contents' array".to_string()
        })?;

    if contents.is_empty() {
        return Ok(Vec::new());
    }

    let text_content = contents[0]
        .get("text")
        .and_then(|t| t.as_str())
        .ok_or_else(|| "Resource contents missing 'text' payload".to_string())?;

    let parsed_json: serde_json::Value = serde_json::from_str(text_content)
        .map_err(|e| format!("Failed to parse unity_instances JSON payload: {e}"))?;

    parse_unity_instances(&parsed_json)
}

/// Parses the instances array/object from CoplayDev `mcpforunity://instances` resource.
pub fn parse_unity_instances(
    raw_json: &serde_json::Value,
) -> Result<Vec<UnityInstanceInfo>, String> {
    let mut instances = Vec::new();
    let items = if let Some(arr) = raw_json.as_array() {
        arr.as_slice()
    } else if let Some(arr) = raw_json.get("instances").and_then(|v| v.as_array()) {
        arr.as_slice()
    } else {
        return Err(
            "Expected instances list to be a JSON array or object with 'instances' array"
                .to_string(),
        );
    };

    for item in items {
        if let Some(s) = item.as_str() {
            let (name, hash) = parse_name_hash(s)?;
            instances.push(UnityInstanceInfo {
                id: s.to_string(),
                name: name.to_string(),
                hash: hash.to_string(),
                is_active: false,
                port: None,
            });
        } else if let Some(obj) = item.as_object() {
            let id_opt = obj.get("id").and_then(|v| v.as_str());
            let name_opt = obj.get("name").and_then(|v| v.as_str());
            let hash_opt = obj.get("hash").and_then(|v| v.as_str());
            let instance_opt = obj.get("instance").and_then(|v| v.as_str());

            let (id, name, hash) = if let Some(id_str) = id_opt {
                if id_str.contains('@') {
                    let (n, h) = parse_name_hash(id_str)?;
                    (id_str.to_string(), n.to_string(), h.to_string())
                } else if let Some(h) = hash_opt {
                    (format!("{id_str}@{h}"), id_str.to_string(), h.to_string())
                } else {
                    return Err(format!(
                        "Instance ID '{id_str}' must be in Name@hash format"
                    ));
                }
            } else if let (Some(n), Some(h)) = (name_opt, hash_opt) {
                (format!("{n}@{h}"), n.to_string(), h.to_string())
            } else if let Some(inst) = instance_opt {
                let (n, h) = parse_name_hash(inst)?;
                (inst.to_string(), n.to_string(), h.to_string())
            } else {
                continue; // Skip unrecognized entry
            };

            let is_active = obj
                .get("is_active")
                .or_else(|| obj.get("isActive"))
                .or_else(|| obj.get("active"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let port = obj.get("port").and_then(|v| v.as_u64()).map(|p| p as u16);

            instances.push(UnityInstanceInfo {
                id,
                name,
                hash,
                is_active,
                port,
            });
        }
    }

    Ok(instances)
}

fn parse_name_hash(s: &str) -> Result<(&str, &str), String> {
    let parts: Vec<&str> = s.split('@').collect();
    if parts.len() != 2 || parts[0].trim().is_empty() || parts[1].trim().is_empty() {
        return Err(format!(
            "Invalid instance identifier '{s}': expected exact 'Name@hash'"
        ));
    }
    Ok((parts[0].trim(), parts[1].trim()))
}

/// Binds an exact Unity instance (`Name@hash`) by executing session-local `set_active_instance`,
/// queries `mcpforunity://editor/state`, and locally verifies that the reported projectRoot
/// canonicalizes and matches an approved workspace project root.
pub async fn bind_unity_instance_core(
    config_dir: &Path,
    session_manager: &McpSessionManager,
    session_id: &str,
    instance_id: &str,
) -> Result<BoundUnityState, String> {
    let trimmed_instance = instance_id.trim();
    if trimmed_instance.is_empty() {
        return Err("Instance ID cannot be empty".to_string());
    }
    parse_name_hash(trimmed_instance)?;

    let (canonical_endpoint, session_hdr, protocol_version, req_id) = session_manager
        .with_session(session_id, |s| {
            Ok((
                s.canonical_endpoint.clone(),
                s.mcp_session_id.clone(),
                s.protocol_version.clone(),
                s.next_request_id(),
            ))
        })
        .await?;

    // Check approval grant
    if let Err(e) = ensure_endpoint_is_approved(config_dir, &canonical_endpoint) {
        session_manager.invalidate_session(session_id).await;
        return Err(e);
    }

    let client = build_http_client()?;

    // Step 1: Call set_active_instance tool
    let tool_req = build_set_active_instance_request(req_id, trimmed_instance);
    let (tool_resp, new_hdr) = match send_jsonrpc_post(
        &client,
        &canonical_endpoint,
        session_hdr.as_deref(),
        Some(&protocol_version),
        &tool_req,
        Some(req_id),
    )
    .await
    {
        Ok(res) => res,
        Err(e) => {
            if e.contains("SESSION_404") {
                session_manager.invalidate_session(session_id).await;
            }
            return Err(e);
        }
    };

    if let Some(h) = new_hdr {
        let _ = session_manager
            .with_session_mut(session_id, |s| {
                s.mcp_session_id = Some(h);
                Ok(())
            })
            .await;
    }

    // Verify tool result for isError
    if let Some(result_obj) = tool_resp.get("result") {
        if result_obj.get("isError").and_then(|v| v.as_bool()) == Some(true) {
            let err_text = result_obj
                .get("content")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|item| item.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("Unknown tool execution error");
            return Err(format!("set_active_instance failed: {err_text}"));
        }
    }

    // Step 2: Query editor state and project info
    let state_req_id = session_manager
        .with_session(session_id, |s| Ok(s.next_request_id()))
        .await?;

    let state_req = build_resources_read_request(state_req_id, "mcpforunity://editor/state");
    let current_session_hdr = session_manager
        .with_session(session_id, |s| Ok(s.mcp_session_id.clone()))
        .await?;

    let (state_resp, state_hdr) = match send_jsonrpc_post(
        &client,
        &canonical_endpoint,
        current_session_hdr.as_deref(),
        Some(&protocol_version),
        &state_req,
        Some(state_req_id),
    )
    .await
    {
        Ok(res) => res,
        Err(e) => {
            if e.contains("SESSION_404") {
                session_manager.invalidate_session(session_id).await;
            }
            return Err(e);
        }
    };

    if let Some(h) = state_hdr {
        let _ = session_manager
            .with_session_mut(session_id, |s| {
                s.mcp_session_id = Some(h);
                Ok(())
            })
            .await;
    }

    let parsed_state = extract_editor_state_json(&state_resp)?;

    let is_playing = parsed_state
        .get("isPlaying")
        .or_else(|| parsed_state.get("is_playing"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let is_paused = parsed_state
        .get("isPaused")
        .or_else(|| parsed_state.get("is_paused"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let is_compiling = parsed_state
        .get("isCompiling")
        .or_else(|| parsed_state.get("is_compiling"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let unity_version = parsed_state
        .get("unityVersion")
        .or_else(|| parsed_state.get("unity_version"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut reported_project_root = parsed_state
        .get("projectRoot")
        .or_else(|| parsed_state.get("project_root"))
        .or_else(|| parsed_state.get("projectPath"))
        .or_else(|| parsed_state.get("project_path"))
        .or_else(|| parsed_state.get("path"))
        .or_else(|| parsed_state.get("rootPath"))
        .or_else(|| parsed_state.get("root_path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // If editor/state did not provide project root, attempt best-effort read from project/info
    if reported_project_root.is_none() {
        let proj_req_id = session_manager
            .with_session(session_id, |s| Ok(s.next_request_id()))
            .await?;
        let proj_req = build_resources_read_request(proj_req_id, "mcpforunity://project/info");
        let cur_hdr = session_manager
            .with_session(session_id, |s| Ok(s.mcp_session_id.clone()))
            .await?;

        if let Ok((proj_resp, _)) = send_jsonrpc_post(
            &client,
            &canonical_endpoint,
            cur_hdr.as_deref(),
            Some(&protocol_version),
            &proj_req,
            Some(proj_req_id),
        )
        .await
        {
            if let Ok(proj_json) = extract_editor_state_json(&proj_resp) {
                reported_project_root = proj_json
                    .get("projectRoot")
                    .or_else(|| proj_json.get("project_root"))
                    .or_else(|| proj_json.get("projectPath"))
                    .or_else(|| proj_json.get("project_path"))
                    .or_else(|| proj_json.get("path"))
                    .or_else(|| proj_json.get("rootPath"))
                    .or_else(|| proj_json.get("root_path"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
            }
        }
    }

    // Step 3: Local project root verification
    let (verification_status, sharing_eligible) =
        verify_project_root_locally(config_dir, reported_project_root.as_deref())?;

    let bound_state = BoundUnityState {
        session_id: session_id.to_string(),
        bound_instance_id: trimmed_instance.to_string(),
        endpoint_url: canonical_endpoint,
        is_playing,
        is_paused,
        is_compiling,
        unity_version,
        project_verification: verification_status,
        provider_sharing_eligible: sharing_eligible,
        warnings: Vec::new(),
    };

    // Update session state in manager
    session_manager
        .with_session_mut(session_id, |s| {
            s.bound_instance_id = Some(trimmed_instance.to_string());
            s.bound_state = Some(bound_state.clone());
            Ok(())
        })
        .await?;

    Ok(bound_state)
}

/// Retrieves the current bound Unity editor state.
pub async fn get_bound_unity_state_core(
    config_dir: &Path,
    session_manager: &McpSessionManager,
    session_id: &str,
) -> Result<BoundUnityState, String> {
    let (canonical_endpoint, bound_instance, session_hdr, protocol_version) = session_manager
        .with_session(session_id, |s| {
            let instance = s.bound_instance_id.clone().ok_or_else(|| {
                "No Unity editor instance has been bound to this session. Please select and bind an instance first."
                    .to_string()
            })?;
            Ok((
                s.canonical_endpoint.clone(),
                instance,
                s.mcp_session_id.clone(),
                s.protocol_version.clone(),
            ))
        })
        .await?;

    // Check approval grant
    if let Err(e) = ensure_endpoint_is_approved(config_dir, &canonical_endpoint) {
        session_manager.invalidate_session(session_id).await;
        return Err(e);
    }

    let client = build_http_client()?;
    let req_id = session_manager
        .with_session(session_id, |s| Ok(s.next_request_id()))
        .await?;
    let state_req = build_resources_read_request(req_id, "mcpforunity://editor/state");

    let (state_resp, state_hdr) = match send_jsonrpc_post(
        &client,
        &canonical_endpoint,
        session_hdr.as_deref(),
        Some(&protocol_version),
        &state_req,
        Some(req_id),
    )
    .await
    {
        Ok(res) => res,
        Err(e) => {
            if e.contains("SESSION_404") {
                session_manager.invalidate_session(session_id).await;
            }
            return Err(e);
        }
    };

    if let Some(h) = state_hdr {
        let _ = session_manager
            .with_session_mut(session_id, |s| {
                s.mcp_session_id = Some(h);
                Ok(())
            })
            .await;
    }

    let parsed_state = extract_editor_state_json(&state_resp)?;

    let is_playing = parsed_state
        .get("isPlaying")
        .or_else(|| parsed_state.get("is_playing"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let is_paused = parsed_state
        .get("isPaused")
        .or_else(|| parsed_state.get("is_paused"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let is_compiling = parsed_state
        .get("isCompiling")
        .or_else(|| parsed_state.get("is_compiling"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let unity_version = parsed_state
        .get("unityVersion")
        .or_else(|| parsed_state.get("unity_version"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let reported_project_root = parsed_state
        .get("projectRoot")
        .or_else(|| parsed_state.get("project_root"))
        .or_else(|| parsed_state.get("projectPath"))
        .or_else(|| parsed_state.get("project_path"))
        .or_else(|| parsed_state.get("path"))
        .or_else(|| parsed_state.get("rootPath"))
        .or_else(|| parsed_state.get("root_path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let (verification_status, sharing_eligible) =
        verify_project_root_locally(config_dir, reported_project_root.as_deref())?;

    let bound_state = BoundUnityState {
        session_id: session_id.to_string(),
        bound_instance_id: bound_instance,
        endpoint_url: canonical_endpoint,
        is_playing,
        is_paused,
        is_compiling,
        unity_version,
        project_verification: verification_status,
        provider_sharing_eligible: sharing_eligible,
        warnings: Vec::new(),
    };

    session_manager
        .with_session_mut(session_id, |s| {
            s.bound_state = Some(bound_state.clone());
            Ok(())
        })
        .await?;

    Ok(bound_state)
}

fn extract_editor_state_json(resp: &serde_json::Value) -> Result<serde_json::Value, String> {
    let contents = resp
        .get("result")
        .and_then(|r| r.get("contents"))
        .and_then(|c| c.as_array())
        .ok_or_else(|| "Missing contents array in editor state response".to_string())?;

    if contents.is_empty() {
        return Ok(serde_json::json!({}));
    }

    let text_content = contents[0]
        .get("text")
        .and_then(|t| t.as_str())
        .ok_or_else(|| "Resource contents missing 'text' payload".to_string())?;

    serde_json::from_str(text_content)
        .map_err(|e| format!("Failed to parse editor state JSON: {e}"))
}

/// Verifies a reported project root against approved workspace project roots.
/// Does NOT infer project root from name alone.
pub fn verify_project_root_locally(
    config_dir: &Path,
    reported_root: Option<&str>,
) -> Result<(UnityProjectVerificationStatus, bool), String> {
    let raw_path = match reported_root {
        Some(p) if !p.trim().is_empty() => p.trim(),
        _ => {
            return Ok((
                UnityProjectVerificationStatus::Unverifiable {
                    reason: "Editor did not provide a projectRoot path. Project root cannot be inferred from instance name alone.".to_string(),
                },
                false,
            ));
        }
    };

    let canonical_reported = match dunce::canonicalize(raw_path) {
        Ok(p) => p,
        Err(e) => {
            return Ok((
                UnityProjectVerificationStatus::Unverifiable {
                    reason: format!(
                        "Reported projectRoot '{raw_path}' cannot be resolved on local disk: {e}"
                    ),
                },
                false,
            ));
        }
    };

    let canonical_reported_str = canonical_reported.to_string_lossy().to_string();
    let settings = load_settings(config_dir)?;

    for approved_root in &settings.project_roots {
        if let Ok(canonical_approved) = dunce::canonicalize(approved_root) {
            if canonical_approved == canonical_reported
                || canonical_reported.starts_with(&canonical_approved)
            {
                let is_shared = settings.provider_sharing_grants.contains(approved_root);
                return Ok((
                    UnityProjectVerificationStatus::Verified {
                        canonical_root: canonical_reported_str,
                        matched_approved_root: approved_root.clone(),
                        provider_sharing_enabled: is_shared,
                    },
                    is_shared,
                ));
            }
        }
    }

    Ok((
        UnityProjectVerificationStatus::Mismatched {
            reported_root: canonical_reported_str,
            reason: "Reported projectRoot does not match any approved workspace project root."
                .to_string(),
        },
        false,
    ))
}

/// Cleanly disconnects an active MCP session (HTTP or Stdio) and removes all session state from memory.
pub async fn disconnect_session_core(
    session_manager: &McpSessionManager,
    session_id: &str,
) -> McpDisconnectResult {
    session_manager.disconnect_any(session_id).await
}
