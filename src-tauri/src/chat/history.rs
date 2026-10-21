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
                    mode: conv.mode,
                    task_root: conv.task_root,
                    task_description: conv.task_description,
                    task_revision: conv.task_revision,
                });
            }
        }
    }

    // Sort by updated_at descending
    list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(list)
}

/// Checks whether a given path is a symlink or reparse point (e.g. NTFS junction, mount point).
pub fn is_symlink_or_reparse(path: &Path) -> Result<bool, std::io::Error> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };

    if meta.file_type().is_symlink() {
        return Ok(true);
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x00000400;
        if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Determines whether a filename matches the requested conversation ID or its
/// recoverable backup/temporary files.
/// Strictly enforces that the filename must begin or contain the full 36-character UUID
/// followed by an extension or backup/tmp delimiter, preventing accidental matches with
/// other conversation IDs, source files, or config files.
pub fn is_matching_conversation_file(file_name: &str, uuid_str: &str) -> bool {
    let lower_name = file_name.to_lowercase();
    let lower_uuid = uuid_str.to_lowercase();

    // Primary or hidden exact matches
    if lower_name == format!("{lower_uuid}.json") || lower_name == format!(".{lower_uuid}.json") {
        return true;
    }

    // Hidden backup or temp files: .{uuid}.bak... or .{uuid}.tmp...
    let dot_prefix = format!(".{lower_uuid}.");
    if lower_name.starts_with(&dot_prefix) {
        let remainder = &lower_name[dot_prefix.len()..];
        if remainder.starts_with("bak") || remainder.starts_with("tmp") {
            return true;
        }
    }

    // Non-hidden backup or temp files: {uuid}.bak... or {uuid}.tmp... or {uuid}.json.bak...
    let plain_prefix = format!("{lower_uuid}.");
    if lower_name.starts_with(&plain_prefix) {
        let remainder = &lower_name[plain_prefix.len()..];
        if remainder.starts_with("bak")
            || remainder.starts_with("tmp")
            || remainder.starts_with("json.bak")
            || remainder.starts_with("json.tmp")
        {
            return true;
        }
    }

    false
}

/// Permanently deletes a single conversation and its associated backup and temporary files from disk.
/// - Validates the UUID format strictly to prevent path traversal or arbitrary filenames.
/// - Requires a valid, configured, existing data root.
/// - Enforces canonical containment within the conversations directory.
/// - Rejects symlinks and reparse points (both for the directory and for individual files).
/// - Deletes all recoverable backup and temp files matching the ID so the conversation cannot resurrect.
/// - Preserves unrelated conversation files, configuration files, and source code.
/// - Is idempotent: if the conversation (or conversations directory) does not exist, succeeds cleanly.
/// - Reports partial filesystem errors rather than returning a false success.
pub fn delete_conversation_from_disk(data_root: &Path, id: &str) -> Result<(), String> {
    let valid_uuid = validate_conversation_id(id)?;
    let uuid_str = valid_uuid.to_string();

    if !data_root.exists() {
        return Err(format!(
            "Data root directory '{}' does not exist or is unavailable.",
            data_root.display()
        ));
    }

    let canonical_data_root = dunce::canonicalize(data_root)
        .map_err(|e| format!("Failed to canonicalize data root: {e}"))?;

    let conv_dir = canonical_data_root.join("conversations");
    if !conv_dir.exists() {
        // Missing conversation is an idempotent success when data root is valid
        return Ok(());
    }

    // Security check: reject symlinks or reparse points on the conversations directory
    if is_symlink_or_reparse(&conv_dir).map_err(|e| {
        format!(
            "Failed to read metadata for conversations directory '{}': {e}",
            conv_dir.display()
        )
    })? {
        return Err(format!(
            "Conversations directory '{}' is a symlink or reparse point, rejecting deletion.",
            conv_dir.display()
        ));
    }

    if !conv_dir.is_dir() {
        return Err(format!(
            "Conversations path '{}' is not a directory.",
            conv_dir.display()
        ));
    }

    let canonical_conv_dir = dunce::canonicalize(&conv_dir)
        .map_err(|e| format!("Failed to canonicalize conversations directory: {e}"))?;

    if !canonical_conv_dir.starts_with(&canonical_data_root) {
        return Err("Conversations directory escapes data root.".to_string());
    }

    // Enumerate candidate files matching the conversation UUID
    let entries = std::fs::read_dir(&canonical_conv_dir)
        .map_err(|e| format!("Failed to read conversations directory: {e}"))?;

    let mut matched_files = Vec::new();
    for entry_res in entries {
        let entry = entry_res.map_err(|e| format!("Failed to read directory entry: {e}"))?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        if is_matching_conversation_file(&file_name, &uuid_str) {
            matched_files.push(entry.path());
        }
    }

    let primary_file = canonical_conv_dir.join(format!("{uuid_str}.json"));
    if primary_file.exists() && !matched_files.contains(&primary_file) {
        matched_files.push(primary_file);
    }

    if matched_files.is_empty() {
        // Missing conversation is an idempotent success
        return Ok(());
    }

    // Pre-deletion validation: canonical containment and rejection of symlinks/reparse points
    for file_path in &matched_files {
        if is_symlink_or_reparse(file_path)
            .map_err(|e| format!("Failed to read metadata for '{}': {e}", file_path.display()))?
        {
            return Err(format!(
                "Conversation file '{}' is a symlink or reparse point, rejecting deletion.",
                file_path.display()
            ));
        }

        if file_path.is_dir() {
            return Err(format!(
                "Conversation target '{}' is a directory, rejecting deletion.",
                file_path.display()
            ));
        }

        let canonical_file = dunce::canonicalize(file_path).map_err(|e| {
            format!(
                "Failed to canonicalize conversation file '{}': {e}",
                file_path.display()
            )
        })?;

        if !canonical_file.starts_with(&canonical_conv_dir)
            || canonical_file.parent() != Some(canonical_conv_dir.as_path())
        {
            return Err(format!(
                "Conversation file '{}' escapes the conversations directory.",
                file_path.display()
            ));
        }
    }

    // Perform deletion and record any filesystem failures
    let mut deletion_errors = Vec::new();
    for file_path in &matched_files {
        if let Err(e) = std::fs::remove_file(file_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                deletion_errors.push(format!("Failed to delete '{}': {e}", file_path.display()));
            }
        }
    }

    if !deletion_errors.is_empty() {
        return Err(format!(
            "Errors encountered while deleting conversation files:\n{}",
            deletion_errors.join("\n")
        ));
    }

    Ok(())
}
