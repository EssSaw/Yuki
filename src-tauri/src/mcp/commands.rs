use tauri::{AppHandle, Manager, State};

use super::client::{
    bind_unity_instance_core, connect_approved_mcp_endpoint_core, disconnect_session_core,
    get_bound_unity_state_core, list_connected_unity_instances_core,
};
use super::discovery::discover_mcp_candidates_core;
use super::grants::{approve_endpoint_impl, get_mcp_grants_impl, revoke_endpoint_impl};
use super::models::{
    BoundUnityState, McpConnectionResult, McpDisconnectResult, McpDiscoveryResult,
    McpEndpointGrant, McpGrantResult, UnityInstanceInfo,
};
use super::session::McpSessionManager;
use super::url::validate_and_canonicalize_loopback_url;

#[tauri::command]
pub async fn list_mcp_candidates(app: AppHandle) -> Result<McpDiscoveryResult, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    discover_mcp_candidates_core(&config_dir, false).await
}

#[tauri::command]
pub async fn discover_mcp_candidates(app: AppHandle) -> Result<McpDiscoveryResult, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    discover_mcp_candidates_core(&config_dir, true).await
}

#[tauri::command]
pub fn approve_mcp_endpoint(app: AppHandle, endpoint: String) -> Result<McpGrantResult, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    approve_endpoint_impl(&config_dir, &endpoint)
}

#[tauri::command]
pub async fn revoke_mcp_endpoint(
    app: AppHandle,
    session_manager: State<'_, McpSessionManager>,
    endpoint: String,
) -> Result<McpGrantResult, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    if let Ok(canonical) = validate_and_canonicalize_loopback_url(&endpoint) {
        session_manager
            .invalidate_endpoint_sessions(&canonical)
            .await;
    }
    revoke_endpoint_impl(&config_dir, &endpoint)
}

#[tauri::command]
pub fn get_mcp_grants(app: AppHandle) -> Result<Vec<McpEndpointGrant>, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    get_mcp_grants_impl(&config_dir)
}

#[tauri::command]
pub async fn connect_approved_mcp_endpoint(
    app: AppHandle,
    session_manager: State<'_, McpSessionManager>,
    endpoint: String,
) -> Result<McpConnectionResult, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    connect_approved_mcp_endpoint_core(&config_dir, &session_manager, &endpoint).await
}

#[tauri::command]
pub async fn list_connected_unity_instances(
    app: AppHandle,
    session_manager: State<'_, McpSessionManager>,
    session_id: String,
) -> Result<Vec<UnityInstanceInfo>, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    list_connected_unity_instances_core(&config_dir, &session_manager, &session_id).await
}

#[tauri::command]
pub async fn bind_unity_instance(
    app: AppHandle,
    session_manager: State<'_, McpSessionManager>,
    session_id: String,
    instance_id: Option<String>,
    exact_id: Option<String>,
) -> Result<BoundUnityState, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let target_id = exact_id
        .or(instance_id)
        .ok_or_else(|| "instance_id or exact_id is required".to_string())?;
    bind_unity_instance_core(&config_dir, &session_manager, &session_id, &target_id).await
}

#[tauri::command]
pub async fn get_bound_unity_state(
    app: AppHandle,
    session_manager: State<'_, McpSessionManager>,
    session_id: String,
) -> Result<BoundUnityState, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    get_bound_unity_state_core(&config_dir, &session_manager, &session_id).await
}

#[tauri::command]
pub async fn disconnect(
    session_manager: State<'_, McpSessionManager>,
    session_id: String,
) -> Result<McpDisconnectResult, String> {
    Ok(disconnect_session_core(&session_manager, &session_id).await)
}

#[tauri::command]
pub async fn disconnect_mcp_session(
    session_manager: State<'_, McpSessionManager>,
    session_id: String,
) -> Result<McpDisconnectResult, String> {
    Ok(disconnect_session_core(&session_manager, &session_id).await)
}
