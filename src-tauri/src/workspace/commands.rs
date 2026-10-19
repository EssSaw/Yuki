use tauri::{AppHandle, Manager};

use super::explorer::list_project_directory_impl;
use super::models::{FileChunk, ProjectEntry, WorkspaceSettings};
use super::reader::read_local_file_chunk_impl;
use super::settings::{
    add_project_root_path, choose_data_root_path, get_workspace_settings_impl,
    remove_project_root_impl, toggle_project_provider_sharing_impl,
};

#[tauri::command]
pub fn get_workspace_settings(app: AppHandle) -> Result<WorkspaceSettings, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    get_workspace_settings_impl(&config_dir)
}

#[tauri::command]
pub async fn choose_data_root(app: AppHandle) -> Result<WorkspaceSettings, String> {
    let picked = tauri::async_runtime::spawn_blocking(|| rfd::FileDialog::new().pick_folder())
        .await
        .map_err(|e| format!("Failed to open folder picker: {e}"))?;

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;

    match picked {
        Some(folder) => choose_data_root_path(&config_dir, &folder),
        None => get_workspace_settings_impl(&config_dir),
    }
}

#[tauri::command]
pub async fn add_project_root(app: AppHandle) -> Result<WorkspaceSettings, String> {
    let picked = tauri::async_runtime::spawn_blocking(|| rfd::FileDialog::new().pick_folder())
        .await
        .map_err(|e| format!("Failed to open folder picker: {e}"))?;

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;

    match picked {
        Some(folder) => add_project_root_path(&config_dir, &folder),
        None => get_workspace_settings_impl(&config_dir),
    }
}

#[tauri::command]
pub fn remove_project_root(app: AppHandle, root: String) -> Result<WorkspaceSettings, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    remove_project_root_impl(&config_dir, &root)
}

#[tauri::command]
pub fn toggle_project_provider_sharing(
    app: AppHandle,
    root: String,
    enabled: Option<bool>,
) -> Result<WorkspaceSettings, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    toggle_project_provider_sharing_impl(&config_dir, &root, enabled)
}

#[tauri::command]
pub fn set_project_provider_sharing(
    app: AppHandle,
    root: String,
    enabled: bool,
) -> Result<WorkspaceSettings, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    toggle_project_provider_sharing_impl(&config_dir, &root, Some(enabled))
}

#[tauri::command]
pub fn list_project_directory(
    app: AppHandle,
    root: String,
    path: Option<String>,
) -> Result<Vec<ProjectEntry>, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let rel_path = path.as_deref().unwrap_or("");
    list_project_directory_impl(&config_dir, &root, rel_path)
}

#[tauri::command]
pub fn read_local_file_chunk(
    app: AppHandle,
    path: String,
    offset: u64,
    chunk_size: Option<u32>,
) -> Result<FileChunk, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    read_local_file_chunk_impl(&config_dir, &path, offset, chunk_size)
}
