use std::io::Write;
use std::path::{Path, PathBuf};

use super::explorer::validate_project_root;
use super::models::WorkspaceSettings;

/// Resolves the dedicated "Yuki" subdirectory for a chosen data folder.
/// Avoids doubling if the selected folder's basename is already "Yuki" (case-insensitive).
pub fn resolve_data_root_subdir(chosen: &Path) -> PathBuf {
    let is_yuki = chosen
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.eq_ignore_ascii_case("Yuki"))
        .unwrap_or(false);

    if is_yuki {
        chosen.to_path_buf()
    } else {
        chosen.join("Yuki")
    }
}

/// Probes whether a directory is writable by creating a unique temporary file with
/// `create_new(true)` and immediately deleting it.
pub fn probe_writable(dir: &Path) -> Result<(), String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let probe_file = dir.join(format!(
        ".yuki_write_test_{}_{}.tmp",
        std::process::id(),
        nanos
    ));

    let f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_file)
        .map_err(|e| format!("Data root directory is not writable: {e}"))?;
    drop(f);

    std::fs::remove_file(&probe_file)
        .map_err(|e| format!("Failed to clean up write probe in data root: {e}"))?;

    Ok(())
}

/// Atomically saves workspace settings to `settings.json` within the specified config directory.
/// Uses a temporary file in the same directory and handles Windows atomic replacement via
/// rollback rename without deleting the existing original file first.
pub fn save_settings_atomic(config_dir: &Path, settings: &WorkspaceSettings) -> Result<(), String> {
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("Failed to create settings directory: {e}"))?;

    let dest = config_dir.join("settings.json");
    let json_bytes = serde_json::to_vec_pretty(settings)
        .map_err(|e| format!("Failed to serialize settings: {e}"))?;

    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let temp_path = config_dir.join(format!(".settings.json.tmp_{}_{}", pid, nanos));
    let backup_path = config_dir.join(format!(".settings.json.bak_{}_{}", pid, nanos));

    // 1. Write to temp file and sync to durable storage
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|e| format!("Failed to create temporary settings file: {e}"))?;
        file.write_all(&json_bytes)
            .map_err(|e| format!("Failed to write settings: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("Failed to sync settings file: {e}"))?;
    }

    // 2. Perform atomic replacement with rollback on Windows
    if dest.exists() {
        std::fs::rename(&dest, &backup_path).map_err(|e| {
            let _ = std::fs::remove_file(&temp_path);
            format!("Failed to move existing settings to backup: {e}")
        })?;

        if let Err(e) = std::fs::rename(&temp_path, &dest) {
            // Rollback original file from backup
            let _ = std::fs::rename(&backup_path, &dest);
            let _ = std::fs::remove_file(&temp_path);
            return Err(format!("Failed to replace settings file: {e}"));
        }

        // Successfully replaced; clean up backup
        let _ = std::fs::remove_file(&backup_path);
    } else if let Err(e) = std::fs::rename(&temp_path, &dest) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Failed to save settings: {e}"));
    }

    Ok(())
}

/// Loads workspace settings from the config directory. If the file does not exist,
/// returns default settings.
pub fn load_settings(config_dir: &Path) -> Result<WorkspaceSettings, String> {
    let dest = config_dir.join("settings.json");
    if !dest.exists() {
        // Attempt recovery from any lingering backup file
        if let Ok(entries) = std::fs::read_dir(config_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(".settings.json.bak_") {
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        if let Ok(mut settings) =
                            serde_json::from_str::<WorkspaceSettings>(&content)
                        {
                            settings.provider_sharing_grants.retain(|grant| {
                                settings.project_roots.iter().any(|root| {
                                    grant == root
                                        || match (
                                            dunce::canonicalize(grant),
                                            dunce::canonicalize(root),
                                        ) {
                                            (Ok(g), Ok(r)) => g == r,
                                            _ => false,
                                        }
                                })
                            });
                            return Ok(settings);
                        }
                    }
                }
            }
        }
        return Ok(WorkspaceSettings::default());
    }

    let content =
        std::fs::read_to_string(&dest).map_err(|e| format!("Failed to read settings file: {e}"))?;

    let mut settings: WorkspaceSettings = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse settings JSON: {e}"))?;

    settings.provider_sharing_grants.retain(|grant| {
        settings.project_roots.iter().any(|root| {
            grant == root
                || match (dunce::canonicalize(grant), dunce::canonicalize(root)) {
                    (Ok(g), Ok(r)) => g == r,
                    _ => false,
                }
        })
    });

    Ok(settings)
}

/// Retrieves the current workspace settings.
pub fn get_workspace_settings_impl(config_dir: &Path) -> Result<WorkspaceSettings, String> {
    load_settings(config_dir)
}

/// Handles choosing a data root given a picked directory path.
/// - Resolves dedicated Yuki subdir.
/// - Creates folder and probes writability.
/// - Checks same-path no-op.
/// - Rejects data-root change if the existing data root contains any entry.
/// - Atomically updates and persists settings.
pub fn choose_data_root_path(
    config_dir: &Path,
    picked_folder: &Path,
) -> Result<WorkspaceSettings, String> {
    let parent = dunce::canonicalize(picked_folder)
        .map_err(|e| format!("Cannot inspect selected data folder: {e}"))?;
    if super::explorer::is_system_directory(&parent) {
        return Err("Choose a data folder outside system and application directories.".into());
    }
    let target_root = resolve_data_root_subdir(&parent);
    let mut settings = load_settings(config_dir)?;
    if let Some(existing) = &settings.data_root {
        let canonical_existing = dunce::canonicalize(existing)
            .map_err(|e| format!("Cannot inspect current data folder; location unchanged: {e}"))?;
        if dunce::canonicalize(&target_root).ok().as_ref() == Some(&canonical_existing) {
            return Ok(settings);
        }
        let mut entries = std::fs::read_dir(&canonical_existing)
            .map_err(|e| format!("Cannot inspect current data folder; location unchanged: {e}"))?;
        if entries.next().is_some() {
            return Err("Cannot change data root because the current data folder is not empty. Relocation is not yet supported.".into());
        }
    }

    std::fs::create_dir_all(&target_root)
        .map_err(|e| format!("Failed to create data directory: {e}"))?;

    let canonical_target = dunce::canonicalize(&target_root)
        .map_err(|e| format!("Failed to canonicalize data directory: {e}"))?;

    probe_writable(&canonical_target)?;

    if super::explorer::is_system_directory(&canonical_target) {
        return Err("Choose a data folder outside system and application directories.".into());
    }
    let target_str = canonical_target.to_string_lossy().to_string();

    // Same path no-op
    if settings.data_root.as_deref() == Some(&target_str) {
        return Ok(settings);
    }
    if let Some(ref existing) = settings.data_root {
        if let Ok(canon_existing) = dunce::canonicalize(existing) {
            if canon_existing == canonical_target {
                return Ok(settings);
            }
        }
    }

    settings.data_root = Some(target_str);
    save_settings_atomic(config_dir, &settings)?;

    Ok(settings)
}

/// Handles adding an approved project root given a picked directory path.
/// - Validates and canonicalizes the path.
/// - Rejects drive roots, user profile roots, and system directories.
/// - Prevents duplicates.
/// - Atomically persists settings.
pub fn add_project_root_path(
    config_dir: &Path,
    picked_folder: &Path,
) -> Result<WorkspaceSettings, String> {
    let canonical = validate_project_root(picked_folder)?;
    let root_str = canonical.to_string_lossy().to_string();

    let mut settings = load_settings(config_dir)?;

    let already_present = settings.project_roots.iter().any(|r| {
        r == &root_str
            || dunce::canonicalize(r)
                .map(|c| c == canonical)
                .unwrap_or(false)
    });

    if already_present {
        return Ok(settings);
    }

    settings.project_roots.push(root_str);
    save_settings_atomic(config_dir, &settings)?;

    Ok(settings)
}

/// Revokes approval for a project root. Does NOT delete any project files on disk.
/// Automatically clears any provider-sharing grant associated with the revoked root.
pub fn remove_project_root_impl(
    config_dir: &Path,
    root_to_remove: &str,
) -> Result<WorkspaceSettings, String> {
    let mut settings = load_settings(config_dir)?;

    let target_canon = dunce::canonicalize(root_to_remove).ok();
    settings.project_roots.retain(|r| {
        if r == root_to_remove {
            return false;
        }
        if let Some(ref tc) = target_canon {
            if let Ok(rc) = dunce::canonicalize(r) {
                if &rc == tc {
                    return false;
                }
            }
        }
        true
    });

    // Clear provider sharing grant for the revoked root
    settings.provider_sharing_grants.retain(|g| {
        if g == root_to_remove {
            return false;
        }
        if let Some(ref tc) = target_canon {
            if let Ok(gc) = dunce::canonicalize(g) {
                if &gc == tc {
                    return false;
                }
            }
        }
        true
    });

    save_settings_atomic(config_dir, &settings)?;

    Ok(settings)
}

/// Toggles or explicitly sets provider sharing for an approved project root.
/// - Validates that the specified root is currently an approved project root.
/// - If `enabled` is Some(true), adds the canonical root to provider_sharing_grants if not already present.
/// - If `enabled` is Some(false), removes the canonical root from provider_sharing_grants.
/// - If `enabled` is None, flips the current grant status.
/// - Atomically persists settings and returns the updated WorkspaceSettings.
pub fn toggle_project_provider_sharing_impl(
    config_dir: &Path,
    root: &str,
    enabled: Option<bool>,
) -> Result<WorkspaceSettings, String> {
    let mut settings = load_settings(config_dir)?;

    let canonical_root =
        dunce::canonicalize(root).map_err(|e| format!("Cannot inspect project root: {e}"))?;
    let canonical_root_str = canonical_root.to_string_lossy().to_string();

    // Verify root is an approved project root
    let is_approved = settings.project_roots.iter().any(|r| {
        r == root
            || dunce::canonicalize(r)
                .map(|c| c == canonical_root)
                .unwrap_or(false)
    });

    if !is_approved {
        return Err("Cannot configure provider sharing for unapproved project folder".to_string());
    }

    let is_currently_granted = settings.provider_sharing_grants.iter().any(|g| {
        g == root
            || g == &canonical_root_str
            || dunce::canonicalize(g)
                .map(|c| c == canonical_root)
                .unwrap_or(false)
    });

    let new_state = match enabled {
        Some(val) => val,
        None => !is_currently_granted,
    };

    if new_state {
        if !is_currently_granted {
            settings.provider_sharing_grants.push(canonical_root_str);
        }
    } else {
        settings.provider_sharing_grants.retain(|g| {
            if g == root || g == &canonical_root_str {
                return false;
            }
            if let Ok(gc) = dunce::canonicalize(g) {
                if gc == canonical_root {
                    return false;
                }
            }
            true
        });
    }

    save_settings_atomic(config_dir, &settings)?;

    Ok(settings)
}
