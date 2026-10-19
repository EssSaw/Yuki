use tauri::{AppHandle, Manager};

use super::explorer::list_project_directory_impl;
use super::models::{ProjectEntry, WorkspaceSettings};
use super::settings::{
    add_project_root_path, choose_data_root_path, get_workspace_settings_impl,
    remove_project_root_impl,
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
pub fn list_project_directory(
    app: AppHandle,
    root: String,
    path: Option<String>,
) -> Result<Vec<ProjectEntry>, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let rel_path = path.as_deref().unwrap_or("");
    list_project_directory_impl(&config_dir, &root, rel_path)
}
