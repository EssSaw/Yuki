use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::workspace::explorer::{is_dos_device_name, is_unc_or_device_path};
use crate::workspace::settings::load_settings;

use super::models::{
    CURRENT_NOTE_SCHEMA_VERSION, DiskNoteRecord, MAX_NOTE_BODY_BYTES, MAX_NOTE_RECORD_BYTES,
    MAX_NOTE_TITLE_CHARS, NoteError, NoteRecord, NoteSummary,
};

/// Global mutex to strictly serialize note write and atomic replacement operations within the application process.
static WRITE_MUTEX: Mutex<()> = Mutex::new(());

/// Computes a deterministic optimistic concurrency version token from raw on-disk file bytes.
/// Uses 64-bit FNV-1a paired with the exact byte count, ensuring any byte change (even external
/// edits via text editors or external tools) produces a completely different token.
pub fn compute_version_token(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}:{:x}", hash, bytes.len())
}

/// Validates that an ID string is a strictly canonical UUID v4.
/// Rejects path traversal, slashes, backslashes, DOS device names, and non-v4 UUIDs.
pub fn validate_note_id(id: &str) -> Result<uuid::Uuid, NoteError> {
    let trimmed = id.trim();
    if trimmed.is_empty() || trimmed.len() != 36 {
        return Err(NoteError::InvalidUuid(
            "Note ID must be a 36-character UUID string".into(),
        ));
    }

    // Only allow lowercase hex and hyphens
    if trimmed.chars().any(|c| !c.is_ascii_hexdigit() && c != '-') {
        return Err(NoteError::InvalidUuid(
            "Note ID contains invalid characters; must contain only hex digits and hyphens".into(),
        ));
    }

    if is_dos_device_name(trimmed) {
        return Err(NoteError::InvalidUuid(
            "Note ID matches a reserved device name".into(),
        ));
    }

    let parsed = uuid::Uuid::parse_str(trimmed)
        .map_err(|e| NoteError::InvalidUuid(format!("Failed to parse note UUID: {e}")))?;

    if parsed.get_version() != Some(uuid::Version::Random) {
        return Err(NoteError::InvalidUuid(
            "Note ID must be a UUID v4 (random UUID)".into(),
        ));
    }

    if parsed.to_string() != id {
        return Err(NoteError::InvalidUuid(
            "Note ID must be canonical lowercase hyphenated UUID format".into(),
        ));
    }

    Ok(parsed)
}

/// Checks that a file or directory is not a symbolic link or Windows NTFS reparse point/junction.
pub fn check_no_symlink_or_reparse(path: &Path) -> Result<(), NoteError> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(NoteError::IoError(format!(
                "Failed to inspect metadata for '{}': {e}",
                path.display()
            )));
        }
    };

    if meta.file_type().is_symlink() {
        return Err(NoteError::SecurityViolation(format!(
            "Symlink rejected at '{}'",
            path.display()
        )));
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return Err(NoteError::SecurityViolation(format!(
                "Reparse point or directory junction rejected at '{}'",
                path.display()
            )));
        }
    }

    Ok(())
}

/// Validates the configured data root directory and all its parent ancestors.
/// Rejects UNC paths, DOS device names, nonexistent paths, non-directories,
/// and any path where the root or any ancestor is a symlink or reparse point.
pub fn validate_data_root(data_root: &Path) -> Result<PathBuf, NoteError> {
    if is_unc_or_device_path(data_root) {
        return Err(NoteError::SecurityViolation(
            "Device and UNC paths are not permitted for data root".into(),
        ));
    }

    if !data_root.exists() {
        return Err(NoteError::DataRootNotFound(format!(
            "Data root directory does not exist: {}",
            data_root.display()
        )));
    }

    if !data_root.is_dir() {
        return Err(NoteError::DataRootInvalid(format!(
            "Data root path is not a directory: {}",
            data_root.display()
        )));
    }

    // Inspect all ancestors up to drive/filesystem root for symlinks and reparse points
    for ancestor in data_root.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        check_no_symlink_or_reparse(ancestor)?;
    }

    let canonical = dunce::canonicalize(data_root)
        .map_err(|e| NoteError::IoError(format!("Failed to canonicalize data root: {e}")))?;

    if !canonical.is_dir() {
        return Err(NoteError::DataRootInvalid(
            "Canonical data root is not a directory".into(),
        ));
    }

    Ok(canonical)
}

/// Resolves and ensures existence of the dedicated `notes` directory under `data_root`.
pub fn resolve_notes_dir(data_root: &Path) -> Result<PathBuf, NoteError> {
    let canonical_root = validate_data_root(data_root)?;
    let notes_dir = canonical_root.join("notes");

    if notes_dir.exists() {
        check_no_symlink_or_reparse(&notes_dir)?;
        if !notes_dir.is_dir() {
            return Err(NoteError::DataRootInvalid(format!(
                "Notes path '{}' exists but is not a directory",
                notes_dir.display()
            )));
        }
    } else {
        std::fs::create_dir_all(&notes_dir).map_err(|e| {
            NoteError::IoError(format!(
                "Failed to create notes directory at '{}': {e}",
                notes_dir.display()
            ))
        })?;
        check_no_symlink_or_reparse(&notes_dir)?;
    }

    dunce::canonicalize(&notes_dir)
        .map_err(|e| NoteError::IoError(format!("Failed to canonicalize notes directory: {e}")))
}

/// Resolves the absolute path for `{uuid}.json` within `notes_dir`.
pub fn note_file_path(notes_dir: &Path, uuid: &uuid::Uuid) -> PathBuf {
    notes_dir.join(format!("{}.json", uuid))
}

/// Validates input limits before attempting any file writes or allocations.
pub fn validate_input_limits(title: &str, body: &str) -> Result<(), NoteError> {
    let title_chars = title.chars().count();
    if title_chars > MAX_NOTE_TITLE_CHARS {
        return Err(NoteError::TitleTooLong {
            limit: MAX_NOTE_TITLE_CHARS,
            actual: title_chars,
        });
    }

    let body_bytes = body.len();
    if body_bytes > MAX_NOTE_BODY_BYTES {
        return Err(NoteError::PayloadTooLarge {
            limit: MAX_NOTE_BODY_BYTES,
            actual: body_bytes,
        });
    }

    Ok(())
}

/// Discovers lingering backup files corresponding to a specific note UUID.
pub fn find_backups_for_note(
    notes_dir: &Path,
    uuid: &uuid::Uuid,
) -> Result<Vec<PathBuf>, NoteError> {
    let prefix = format!(".{}.json.bak_", uuid);
    let mut backups = Vec::new();

    let read_dir = match std::fs::read_dir(notes_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(backups),
        Err(e) => {
            return Err(NoteError::IoError(format!(
                "Failed to read notes directory: {e}"
            )));
        }
    };

    for entry in read_dir {
        let entry = entry
            .map_err(|e| NoteError::IoError(format!("Failed to read directory entry: {e}")))?;
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();
        if name_str.starts_with(&prefix) {
            backups.push(entry.path());
        }
    }

    backups.sort();
    Ok(backups)
}

/// Performs conservative recovery for a note if its primary file is missing.
/// - If `{uuid}.json` exists: no-op.
/// - If `{uuid}.json` is missing and exactly 1 valid backup file exists: restore backup to `{uuid}.json`.
/// - If `{uuid}.json` is missing and multiple backup files exist: preserve all files and return AmbiguousRecovery.
/// - If backup file is corrupt or future schema: preserve file without recovery.
pub fn recover_note_if_needed(notes_dir: &Path, uuid: &uuid::Uuid) -> Result<(), NoteError> {
    let dest = note_file_path(notes_dir, uuid);
    if dest.exists() {
        return Ok(());
    }

    let backups = find_backups_for_note(notes_dir, uuid)?;
    if backups.is_empty() {
        return Ok(());
    }

    if backups.len() > 1 {
        return Err(NoteError::AmbiguousRecovery(format!(
            "Multiple backup files ({}) found for note '{}' while primary file is missing. Files preserved without automatic recovery to prevent data loss.",
            backups.len(),
            uuid
        )));
    }

    let backup_path = &backups[0];
    check_no_symlink_or_reparse(backup_path)?;

    let bytes = std::fs::read(backup_path).map_err(|e| {
        NoteError::IoError(format!(
            "Failed to read backup note file '{}': {e}",
            backup_path.display()
        ))
    })?;

    let parsed: DiskNoteRecord = serde_json::from_slice(&bytes).map_err(|e| {
        NoteError::CorruptData(format!(
            "Backup note file '{}' contains corrupt JSON: {e}. Preserving file without recovery.",
            backup_path.display()
        ))
    })?;

    if parsed.schema_version > CURRENT_NOTE_SCHEMA_VERSION {
        return Err(NoteError::FutureSchemaVersion {
            expected: CURRENT_NOTE_SCHEMA_VERSION,
            actual: parsed.schema_version,
        });
    }

    if parsed.id != uuid.to_string() {
        return Err(NoteError::CorruptData(format!(
            "Backup note record ID '{}' does not match expected UUID '{}'. Preserving file.",
            parsed.id, uuid
        )));
    }

    std::fs::rename(backup_path, &dest).map_err(|e| {
        NoteError::IoError(format!(
            "Failed to restore note from backup '{}' to '{}': {e}",
            backup_path.display(),
            dest.display()
        ))
    })?;

    Ok(())
}

/// Creates a new Markdown note under the specified data root.
/// Generates a stable UUID v4 id, stores a single versioned JSON record,
/// and returns the complete `NoteRecord` with its initial version token.
pub fn create_note_impl(
    data_root: &Path,
    title: &str,
    body: &str,
) -> Result<NoteRecord, NoteError> {
    let _guard = WRITE_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    validate_input_limits(title, body)?;

    let notes_dir = resolve_notes_dir(data_root)?;
    let uuid = uuid::Uuid::new_v4();
    let dest = note_file_path(&notes_dir, &uuid);

    if dest.exists() {
        return Err(NoteError::NoteAlreadyExists(format!(
            "Note with ID '{}' already exists",
            uuid
        )));
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let disk_record = DiskNoteRecord {
        schema_version: CURRENT_NOTE_SCHEMA_VERSION,
        id: uuid.to_string(),
        title: title.to_string(),
        body: body.to_string(),
        created_at: now.clone(),
        updated_at: now,
    };

    let json_bytes = serde_json::to_vec_pretty(&disk_record).map_err(|e| {
        NoteError::SerializationError(format!("Failed to serialize note record: {e}"))
    })?;

    if json_bytes.len() > MAX_NOTE_RECORD_BYTES {
        return Err(NoteError::PayloadTooLarge {
            limit: MAX_NOTE_RECORD_BYTES,
            actual: json_bytes.len(),
        });
    }

    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let temp_path = notes_dir.join(format!(".{}.json.tmp_{}_{}", uuid, pid, nanos));

    // 1. Write to temp file with create_new(true) and sync
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|e| {
                NoteError::IoError(format!("Failed to create temporary note file: {e}"))
            })?;

        if let Err(e) = file.write_all(&json_bytes) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(NoteError::IoError(format!(
                "Failed to write temporary note file: {e}"
            )));
        }

        if let Err(e) = file.sync_all() {
            let _ = std::fs::remove_file(&temp_path);
            return Err(NoteError::IoError(format!(
                "Failed to sync temporary note file: {e}"
            )));
        }
    }

    // 2. Finalize creation by moving temp file to destination
    if let Err(e) = std::fs::rename(&temp_path, &dest) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(NoteError::IoError(format!(
            "Failed to finalize note creation: {e}"
        )));
    }

    let version_token = compute_version_token(&json_bytes);
    Ok(disk_record.into_record(version_token))
}

/// Loads a note record by UUID from the configured data root.
/// Returns the full `NoteRecord` and computed optimistic version token.
pub fn load_note_impl(data_root: &Path, id: &str) -> Result<NoteRecord, NoteError> {
    let uuid = validate_note_id(id)?;
    let notes_dir = resolve_notes_dir(data_root)?;
    let dest = note_file_path(&notes_dir, &uuid);

    if !dest.exists() {
        let _guard = WRITE_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        recover_note_if_needed(&notes_dir, &uuid)?;
    }

    if !dest.exists() {
        return Err(NoteError::NoteNotFound(format!(
            "Note with ID '{}' was not found",
            id
        )));
    }

    check_no_symlink_or_reparse(&dest)?;

    let bytes = std::fs::read(&dest).map_err(|e| {
        NoteError::IoError(format!(
            "Failed to read note file '{}': {e}",
            dest.display()
        ))
    })?;

    let disk_record: DiskNoteRecord = serde_json::from_slice(&bytes).map_err(|e| {
        NoteError::CorruptData(format!(
            "Note file '{}' contains corrupt data: {e}",
            dest.display()
        ))
    })?;

    if disk_record.schema_version > CURRENT_NOTE_SCHEMA_VERSION {
        return Err(NoteError::FutureSchemaVersion {
            expected: CURRENT_NOTE_SCHEMA_VERSION,
            actual: disk_record.schema_version,
        });
    }

    if disk_record.id != uuid.to_string() {
        return Err(NoteError::CorruptData(format!(
            "Note record ID '{}' does not match file UUID '{}'",
            disk_record.id, uuid
        )));
    }

    let version_token = compute_version_token(&bytes);
    Ok(disk_record.into_record(version_token))
}

/// Saves an existing note (updating title and/or body) with optimistic concurrency.
/// - Requires `expected_version` matching current file bytes; rejects conflicts.
/// - Renaming display title does NOT rename the file on disk (UUID remains stable).
/// - Enforces limits before any disk writes.
/// - Never overwrites corrupt data or future schema version records.
/// - Writes temp file -> fsync -> backup destination -> rename temp to dest -> remove backup.
/// - On rename failure, rolls back from backup to preserve the original note.
pub fn save_note_impl(
    data_root: &Path,
    id: &str,
    title: &str,
    body: &str,
    expected_version: &str,
) -> Result<NoteRecord, NoteError> {
    let _guard = WRITE_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    validate_input_limits(title, body)?;

    let uuid = validate_note_id(id)?;
    let notes_dir = resolve_notes_dir(data_root)?;
    let dest = note_file_path(&notes_dir, &uuid);

    if !dest.exists() {
        recover_note_if_needed(&notes_dir, &uuid)?;
    }

    if !dest.exists() {
        return Err(NoteError::NoteNotFound(format!(
            "Note with ID '{}' was not found for update",
            id
        )));
    }

    check_no_symlink_or_reparse(&dest)?;

    let existing_bytes = std::fs::read(&dest)
        .map_err(|e| NoteError::IoError(format!("Failed to read existing note file: {e}")))?;

    // Inspect existing record; NEVER overwrite corrupt or future schema data
    let existing_record: DiskNoteRecord = match serde_json::from_slice(&existing_bytes) {
        Ok(r) => r,
        Err(e) => {
            return Err(NoteError::CorruptData(format!(
                "Existing note file '{}' contains corrupt data: {e}. Preserving original file without modification.",
                dest.display()
            )));
        }
    };

    if existing_record.schema_version > CURRENT_NOTE_SCHEMA_VERSION {
        return Err(NoteError::FutureSchemaVersion {
            expected: CURRENT_NOTE_SCHEMA_VERSION,
            actual: existing_record.schema_version,
        });
    }

    // Optimistic concurrency check: detects any external byte modifications
    let current_version = compute_version_token(&existing_bytes);
    if current_version != expected_version {
        return Err(NoteError::VersionConflict {
            expected: expected_version.to_string(),
            actual: current_version,
        });
    }

    let updated_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let new_record = DiskNoteRecord {
        schema_version: CURRENT_NOTE_SCHEMA_VERSION,
        id: uuid.to_string(),
        title: title.to_string(),
        body: body.to_string(),
        created_at: existing_record.created_at, // Preserve original creation timestamp
        updated_at,
    };

    let json_bytes = serde_json::to_vec_pretty(&new_record).map_err(|e| {
        NoteError::SerializationError(format!("Failed to serialize note record: {e}"))
    })?;

    if json_bytes.len() > MAX_NOTE_RECORD_BYTES {
        return Err(NoteError::PayloadTooLarge {
            limit: MAX_NOTE_RECORD_BYTES,
            actual: json_bytes.len(),
        });
    }

    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let temp_path = notes_dir.join(format!(".{}.json.tmp_{}_{}", uuid, pid, nanos));
    let backup_path = notes_dir.join(format!(".{}.json.bak_{}_{}", uuid, pid, nanos));

    // 1. Write to temp file with create_new(true) and sync to durable storage
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|e| {
                NoteError::IoError(format!("Failed to create temporary note file: {e}"))
            })?;

        if let Err(e) = file.write_all(&json_bytes) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(NoteError::IoError(format!(
                "Failed to write temporary note file: {e}"
            )));
        }

        if let Err(e) = file.sync_all() {
            let _ = std::fs::remove_file(&temp_path);
            return Err(NoteError::IoError(format!(
                "Failed to sync temporary note file: {e}"
            )));
        }
    }

    // 2. Move existing note to backup
    std::fs::rename(&dest, &backup_path).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        NoteError::IoError(format!("Failed to move existing note to backup: {e}"))
    })?;

    // 3. Move temp file to destination
    if let Err(e) = std::fs::rename(&temp_path, &dest) {
        // Rollback original file from backup
        let rollback_res = std::fs::rename(&backup_path, &dest);
        let _ = std::fs::remove_file(&temp_path);

        if let Err(rb_err) = rollback_res {
            return Err(NoteError::AmbiguousRecovery(format!(
                "Failed to replace note file ({e}) AND rollback from backup failed ({rb_err}). Original note preserved at '{}'",
                backup_path.display()
            )));
        }

        return Err(NoteError::IoError(format!(
            "Failed to replace note file: {e}. Original note successfully restored from backup."
        )));
    }

    // 4. Success: remove backup
    let _ = std::fs::remove_file(&backup_path);

    let version_token = compute_version_token(&json_bytes);
    Ok(new_record.into_record(version_token))
}

/// Lists all note summaries under the configured data root.
/// Returns summaries sorted by `updated_at` descending (most recently modified first).
pub fn list_notes_impl(data_root: &Path) -> Result<Vec<NoteSummary>, NoteError> {
    let canonical_root = validate_data_root(data_root)?;
    let notes_dir = canonical_root.join("notes");

    if !notes_dir.exists() {
        return Ok(Vec::new());
    }

    check_no_symlink_or_reparse(&notes_dir)?;
    if !notes_dir.is_dir() {
        return Err(NoteError::DataRootInvalid(format!(
            "'{}' is not a directory",
            notes_dir.display()
        )));
    }

    // Scan for orphan backups and attempt conservative recovery
    if let Ok(entries) = std::fs::read_dir(&notes_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') && name.contains(".json.bak_") {
                if let Some(rest) = name.strip_prefix('.') {
                    if let Some(uuid_str) = rest.split('.').next() {
                        if let Ok(uuid) = validate_note_id(uuid_str) {
                            let dest = note_file_path(&notes_dir, &uuid);
                            if !dest.exists() {
                                let _guard = WRITE_MUTEX
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                                let _ = recover_note_if_needed(&notes_dir, &uuid);
                            }
                        }
                    }
                }
            }
        }
    }

    let mut summaries = Vec::new();
    let entries = std::fs::read_dir(&notes_dir)
        .map_err(|e| NoteError::IoError(format!("Failed to read notes directory: {e}")))?;

    for entry_res in entries {
        let entry = entry_res
            .map_err(|e| NoteError::IoError(format!("Failed to read directory entry: {e}")))?;
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();

        if name_str.starts_with('.') || !name_str.ends_with(".json") {
            continue;
        }

        let stem = match name_str.strip_suffix(".json") {
            Some(s) => s,
            None => continue,
        };

        let uuid = match validate_note_id(stem) {
            Ok(u) => u,
            Err(_) => continue,
        };

        let path = entry.path();
        if check_no_symlink_or_reparse(&path).is_err() {
            continue;
        }

        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };

        let disk_record: DiskNoteRecord = match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            Err(_) => continue, // Corrupt files skipped during listing
        };

        if disk_record.schema_version > CURRENT_NOTE_SCHEMA_VERSION {
            continue; // Future schema versions skipped during listing
        }

        if disk_record.id != uuid.to_string() {
            continue;
        }

        let version_token = compute_version_token(&bytes);
        summaries.push(disk_record.to_summary(version_token));
    }

    summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(summaries)
}

/// Loads workspace settings from `config_dir` and resolves the configured `data_root`.
pub fn get_data_root_from_config(config_dir: &Path) -> Result<PathBuf, NoteError> {
    let settings = load_settings(config_dir)
        .map_err(|e| NoteError::IoError(format!("Failed to load workspace settings: {e}")))?;

    match settings.data_root {
        Some(root) if !root.trim().is_empty() => Ok(PathBuf::from(root)),
        _ => Err(NoteError::DataRootNotConfigured(
            "Data root folder is not configured. Please choose a data folder in settings first."
                .into(),
        )),
    }
}

/// Helper connecting `config_dir` settings to `create_note_impl`.
pub fn create_note_with_config(
    config_dir: &Path,
    title: &str,
    body: &str,
) -> Result<NoteRecord, NoteError> {
    let data_root = get_data_root_from_config(config_dir)?;
    create_note_impl(&data_root, title, body)
}

/// Helper connecting `config_dir` settings to `list_notes_impl`.
pub fn list_notes_with_config(config_dir: &Path) -> Result<Vec<NoteSummary>, NoteError> {
    let data_root = get_data_root_from_config(config_dir)?;
    list_notes_impl(&data_root)
}

/// Helper connecting `config_dir` settings to `load_note_impl`.
pub fn load_note_with_config(config_dir: &Path, id: &str) -> Result<NoteRecord, NoteError> {
    let data_root = get_data_root_from_config(config_dir)?;
    load_note_impl(&data_root, id)
}

/// Helper connecting `config_dir` settings to `save_note_impl`.
pub fn save_note_with_config(
    config_dir: &Path,
    id: &str,
    title: &str,
    body: &str,
    expected_version: &str,
) -> Result<NoteRecord, NoteError> {
    let data_root = get_data_root_from_config(config_dir)?;
    save_note_impl(&data_root, id, title, body, expected_version)
}
