use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::models::{ConversationDetail, ConversationSummary};

/// Validates that an ID string is strictly a valid UUID v4 / UUID.
/// Protects against path traversal, arbitrary filenames, or malicious characters.
pub fn validate_conversation_id(id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(id).map_err(|_| "Invalid conversation ID: must be a valid UUID".to_string())
}

/// Resolves and ensures the conversations directory inside data_root exists.
pub fn get_conversations_dir(data_root: &Path) -> Result<PathBuf, String> {
    if !data_root.exists() {
        return Err(format!(
            "Data root directory '{}' does not exist or is unavailable.",
            data_root.display()
        ));
    }
    let dir = data_root.join("conversations");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create conversations directory: {e}"))?;
    Ok(dir)
}

/// Atomically persists a conversation record to disk using a temp file and backup rollback.
pub fn save_conversation_atomic(data_root: &Path, conv: &ConversationDetail) -> Result<(), String> {
    let valid_uuid = validate_conversation_id(&conv.id)?;
    let dir = get_conversations_dir(data_root)?;

    let dest = dir.join(format!("{valid_uuid}.json"));
    let json_bytes = serde_json::to_vec_pretty(conv)
        .map_err(|e| format!("Failed to serialize conversation: {e}"))?;

    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let temp_path = dir.join(format!(".{valid_uuid}.tmp_{pid}_{nanos}"));
    let backup_path = dir.join(format!(".{valid_uuid}.bak_{pid}_{nanos}"));

    // 1. Write to temporary file and flush
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|e| format!("Failed to create temporary conversation file: {e}"))?;
        file.write_all(&json_bytes)
            .map_err(|e| format!("Failed to write conversation: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("Failed to sync conversation file: {e}"))?;
    }

    // 2. Perform atomic replace with rollback on Windows
    if dest.exists() {
        std::fs::rename(&dest, &backup_path).map_err(|e| {
            let _ = std::fs::remove_file(&temp_path);
            format!("Failed to move existing conversation to backup: {e}")
        })?;

        if let Err(e) = std::fs::rename(&temp_path, &dest) {
            let _ = std::fs::rename(&backup_path, &dest);
            let _ = std::fs::remove_file(&temp_path);
            return Err(format!("Failed to replace conversation file: {e}"));
        }

        let _ = std::fs::remove_file(&backup_path);
    } else if let Err(e) = std::fs::rename(&temp_path, &dest) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Failed to save conversation: {e}"));
    }

    Ok(())
}

/// Loads a single conversation by UUID. If the primary file is missing, attempts
/// recovery from a lingering backup file.
pub fn load_conversation_from_disk(
    data_root: &Path,
    id: &str,
) -> Result<ConversationDetail, String> {
    let valid_uuid = validate_conversation_id(id)?;
    let dir = get_conversations_dir(data_root)?;
    let dest = dir.join(format!("{valid_uuid}.json"));

    if !dest.exists() {
        // Check for lingering backup recovery
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let prefix = format!(".{valid_uuid}.bak_");
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&prefix) {
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        if let Ok(conv) = serde_json::from_str::<ConversationDetail>(&content) {
                            return Ok(conv);
                        }
                    }
                }
            }
        }
        return Err(format!("Conversation '{id}' not found."));
    }

    let content = std::fs::read_to_string(&dest)
        .map_err(|e| format!("Failed to read conversation file: {e}"))?;

    serde_json::from_str::<ConversationDetail>(&content)
        .map_err(|e| format!("Failed to parse conversation JSON (file may be corrupted): {e}"))
}

/// Lists all saved conversations from disk, sorted by updatedAt descending.
pub fn list_conversations_from_disk(data_root: &Path) -> Result<Vec<ConversationSummary>, String> {
    let dir = data_root.join("conversations");
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut list = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .map_err(|e| format!("Failed to read conversations directory: {e}"))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        // Ignore hidden, temp, or backup files
        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }

        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };

        // Strictly ensure stem is a valid UUID
        if Uuid::parse_str(stem).is_err() {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(conv) = serde_json::from_str::<ConversationDetail>(&content) {
                list.push(ConversationSummary {
                    id: conv.id,
                    title: conv.title,
                    updated_at: conv.updated_at,
                });
            }
        }
    }

    // Sort by updated_at descending
    list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(list)
}
