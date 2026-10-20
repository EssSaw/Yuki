use tauri::{AppHandle, Manager};

use super::discovery::discover_mcp_candidates_core;
use super::grants::{approve_endpoint_impl, get_mcp_grants_impl, revoke_endpoint_impl};
use super::models::{McpDiscoveryResult, McpEndpointGrant, McpGrantResult};

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
pub fn revoke_mcp_endpoint(app: AppHandle, endpoint: String) -> Result<McpGrantResult, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    revoke_endpoint_impl(&config_dir, &endpoint)
}

#[tauri::command]
pub fn get_mcp_grants(app: AppHandle) -> Result<Vec<McpEndpointGrant>, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    get_mcp_grants_impl(&config_dir)
}
