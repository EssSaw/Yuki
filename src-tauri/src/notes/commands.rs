use tauri::{AppHandle, Manager};

use super::models::{NoteRecord, NoteSummary};
use super::storage::{
    create_note_with_config, list_notes_with_config, load_note_with_config, save_note_with_config,
};

/// Tauri command to create a new Markdown note under the active data root.
#[tauri::command]
pub fn create_note(app: AppHandle, title: String, body: String) -> Result<NoteRecord, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    create_note_with_config(&config_dir, &title, &body).map_err(|e| e.to_string())
}

/// Tauri command to list all note summaries under the active data root.
#[tauri::command]
pub fn list_notes(app: AppHandle) -> Result<Vec<NoteSummary>, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    list_notes_with_config(&config_dir).map_err(|e| e.to_string())
}

/// Tauri command to load a note record by UUID under the active data root.
#[tauri::command]
pub fn load_note(app: AppHandle, id: String) -> Result<NoteRecord, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    load_note_with_config(&config_dir, &id).map_err(|e| e.to_string())
}

/// Tauri command to save/update an existing note record with optimistic concurrency.
#[tauri::command]
pub fn save_note(
    app: AppHandle,
    id: String,
    title: String,
    body: String,
    expected_version: String,
) -> Result<NoteRecord, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    save_note_with_config(&config_dir, &id, &title, &body, &expected_version)
        .map_err(|e| e.to_string())
}
