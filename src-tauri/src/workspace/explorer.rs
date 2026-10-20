use std::path::{Path, PathBuf};

use super::models::ProjectEntry;
use super::settings::load_settings;

/// Checks if a directory name corresponds to a common secret/credential store.
pub fn is_secret_directory(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        ".ssh"
            | ".aws"
            | ".azure"
            | ".kube"
            | ".gnupg"
            | ".docker"
            | ".gcloud"
            | ".password-store"
            | "secrets"
            | ".secrets"
    )
}

/// Checks if a component name matches a reserved Windows DOS device name.
pub fn is_dos_device_name(name: &str) -> bool {
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Detects if a path is a device or UNC path.
pub fn is_unc_or_device_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    if path_str.starts_with(r"\\") || path_str.starts_with("//") {
        return true;
    }
    for comp in path.components() {
        if let std::path::Component::Prefix(prefix) = comp {
            match prefix.kind() {
                std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_) => {}
                _ => return true,
            }
        }
        if let std::path::Component::Normal(name) = comp {
            if is_dos_device_name(&name.to_string_lossy()) {
                return true;
            }
        }
    }
    false
}

/// Checks if a file or directory name is considered secret, sensitive, or system-restricted.
pub fn is_blocked_name(name: &str) -> bool {
    let lower = name.to_lowercase();

    // Secret and credential directories
    if is_secret_directory(name) {
        return true;
    }

    // Git metadata and repositories
    if lower == ".git" {
        return true;
    }

    // Environment and configuration files
    if lower == ".env" || lower.starts_with(".env.") {
        return true;
    }

    // Credentials files
    if lower == "credentials" || lower.starts_with("credentials.") {
        return true;
    }

    // Package manager and auth tokens
    if lower == ".npmrc" || lower == ".pypirc" || lower == ".netrc" || lower == "_netrc" {
        return true;
    }

    // Secrets files
    if lower == "secrets.json"
        || lower == "secrets.yaml"
        || lower == "secrets.yml"
        || lower == "secrets.toml"
        || lower == "secrets.env"
        || lower == "known_hosts"
        || lower == "authorized_keys"
    {
        return true;
    }

    // Private keys and certificates
    if lower.ends_with(".pem") || lower.ends_with(".key") {
        return true;
    }

    // SSH and standard key names
    if lower == "id_rsa"
        || lower.starts_with("id_rsa.")
        || lower == "id_ed25519"
        || lower.starts_with("id_ed25519.")
        || lower == "id_ecdsa"
        || lower.starts_with("id_ecdsa.")
        || lower == "id_dsa"
        || lower.starts_with("id_dsa.")
    {
        return true;
    }

    // Windows system and recycle folders
    if lower == "$recycle.bin"
        || lower == "system volume information"
        || lower == "recovery"
        || lower == "$windows.~bt"
        || lower == "$windows.~ws"
        || lower == "config.msi"
    {
        return true;
    }

    false
}

/// Detects if the path is a whole drive root (e.g. C:\ or /).
pub fn is_drive_root(path: &Path) -> bool {
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Prefix(_)), Some(std::path::Component::RootDir)) => {
            components.next().is_none()
        }
        (Some(std::path::Component::RootDir), None) => true,
        _ => path.parent().is_none(),
    }
}

/// Detects if the path is the user's profile root (e.g. C:\Users\Username or /home/username)
/// or the broad user directory container (e.g. C:\Users or /home).
pub fn is_user_profile_root(path: &Path) -> bool {
    if let Some(user_profile) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
    {
        if let Ok(canon_home) = dunce::canonicalize(user_profile) {
            if path == canon_home {
                return true;
            }
        }
    }

    // Check if path is C:\Users or /home or /Users
    let components: Vec<_> = path.components().collect();
    if components.len() == 3 {
        if let (
            std::path::Component::Prefix(_),
            std::path::Component::RootDir,
            std::path::Component::Normal(dir),
        ) = (&components[0], &components[1], &components[2])
        {
            if dir.to_string_lossy().eq_ignore_ascii_case("Users") {
                return true;
            }
        }
    } else if components.len() == 2 {
        if let (std::path::Component::RootDir, std::path::Component::Normal(dir)) =
            (&components[0], &components[1])
        {
            let s = dir.to_string_lossy();
            if s.eq_ignore_ascii_case("home") || s.eq_ignore_ascii_case("Users") {
                return true;
            }
        }
    }

    false
}

/// Detects if the path is an operating system or critical system directory.
pub fn is_system_directory(path: &Path) -> bool {
    // Windows system environment variables
    let sys_vars = [
        "SystemRoot",
        "windir",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "ProgramData",
    ];

    for var in sys_vars {
        if let Some(val) = std::env::var_os(var) {
            if let Ok(canon_val) = dunce::canonicalize(val) {
                if path.starts_with(&canon_val) {
                    return true;
                }
            }
        }
    }

    // AppData checks: avoid whole AppData or AppData subfolders as project roots
    let appdata_vars = ["APPDATA", "LOCALAPPDATA"];
    for var in appdata_vars {
        if let Some(val) = std::env::var_os(var) {
            if let Ok(canon_val) = dunce::canonicalize(&val) {
                if path == canon_val || path.starts_with(&canon_val) {
                    return true;
                }
                if let Some(parent) = canon_val.parent() {
                    if path == parent {
                        return true;
                    }
                }
            }
        }
    }

    // Check each component for system directory names
    for comp in path.components() {
        if let std::path::Component::Normal(name) = comp {
            let s = name.to_string_lossy();
            let lower = s.to_lowercase();
            if lower == "$recycle.bin"
                || lower == "system volume information"
                || lower == "recovery"
                || lower == "$windows.~bt"
                || lower == "$windows.~ws"
                || lower == "config.msi"
            {
                return true;
            }
        }
    }

    #[cfg(unix)]
    {
        let unix_sys = [
            "/bin", "/boot", "/dev", "/etc", "/lib", "/lib64", "/proc", "/root", "/sbin", "/sys",
            "/usr", "/var",
        ];
        for sys in unix_sys {
            if path.starts_with(sys) {
                return true;
            }
        }
    }

    false
}

/// Validates a project root candidate path and returns its canonicalized PathBuf.
pub fn validate_project_root(path: &Path) -> Result<PathBuf, String> {
    if is_unc_or_device_path(path) {
        return Err("Device and UNC paths are not permitted".to_string());
    }

    if !path.exists() {
        return Err("Project directory does not exist".to_string());
    }

    let canonical =
        dunce::canonicalize(path).map_err(|e| format!("Failed to canonicalize path: {e}"))?;

    if !canonical.is_dir() {
        return Err("Selected path is not a directory".to_string());
    }

    if is_drive_root(&canonical) {
        return Err("Drive root cannot be added as a project folder. Please choose a specific project folder.".to_string());
    }

    if is_user_profile_root(&canonical) {
        return Err("User profile folder cannot be added as a project folder. Please choose a specific project folder under your profile.".to_string());
    }

    if is_system_directory(&canonical) {
        return Err("System directories cannot be added as project folders.".to_string());
    }

    Ok(canonical)
}

/// Lists immediate entries in an approved project directory.
pub fn list_project_directory_impl(
    config_dir: &Path,
    root: &str,
    relative_path: &str,
) -> Result<Vec<ProjectEntry>, String> {
    let settings = load_settings(config_dir)?;

    let canonical_root = dunce::canonicalize(root)
        .map_err(|e| format!("Failed to canonicalize root directory: {e}"))?;

    let is_approved = settings.project_roots.iter().any(|approved| {
        approved == root
            || dunce::canonicalize(approved)
                .map(|c| c == canonical_root)
                .unwrap_or(false)
    });

    if !is_approved {
        return Err("Root directory is not an approved project folder".to_string());
    }

    let trimmed_rel = relative_path.trim();
    if is_unc_or_device_path(Path::new(root)) {
        return Err("Device and UNC paths are not permitted".to_string());
    }
    let (target_dir, clean_prefix) = if trimmed_rel.is_empty() || trimmed_rel == "." {
        (canonical_root.clone(), String::new())
    } else {
        // Validate each component of the relative path
        let mut normal_parts = Vec::new();
        for comp in Path::new(trimmed_rel).components() {
            match comp {
                std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                    return Err("Absolute path components are not allowed".to_string());
                }
                std::path::Component::ParentDir => {
                    return Err("Path traversal (..) is not allowed".to_string());
                }
                std::path::Component::CurDir => continue,
                std::path::Component::Normal(c) => {
                    let s = c.to_string_lossy();
                    if is_dos_device_name(&s) {
                        return Err("Device and UNC paths are not permitted".to_string());
                    }
                    if is_blocked_name(&s) {
                        return Err("Access to restricted folder or file is blocked".to_string());
                    }
                    normal_parts.push(s.to_string());
                }
            }
        }

        let mut current = canonical_root.clone();
        for part in &normal_parts {
            current = current.join(part);
            if let Ok(meta) = std::fs::symlink_metadata(&current) {
                if meta.file_type().is_symlink() {
                    if let Ok(target) = dunce::canonicalize(&current) {
                        if !target.starts_with(&canonical_root) {
                            return Err("Symlink or junction escapes project root".to_string());
                        }
                    }
                }
            }
        }

        let candidate = current;
        if !candidate.exists() {
            return Err("Directory does not exist".to_string());
        }
        if !candidate.is_dir() {
            return Err("Target is not a directory".to_string());
        }

        let canonical_target = dunce::canonicalize(&candidate)
            .map_err(|e| format!("Failed to canonicalize target directory: {e}"))?;

        if !canonical_target.starts_with(&canonical_root) {
            return Err("Directory traversal escape detected".to_string());
        }

        (canonical_target, normal_parts.join("/"))
    };

    let read_dir =
        std::fs::read_dir(&target_dir).map_err(|e| format!("Failed to read directory: {e}"))?;

    let mut entries = Vec::new();
    for entry_res in read_dir {
        let entry = entry_res.map_err(|e| format!("Failed to read entry: {e}"))?;
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy().to_string();

        if is_blocked_name(&name) {
            continue;
        }

        let entry_path = entry.path();
        let symlink_meta = std::fs::symlink_metadata(&entry_path)
            .map_err(|e| format!("Failed to read entry metadata: {e}"))?;

        let is_dir = if symlink_meta.file_type().is_symlink() {
            match dunce::canonicalize(&entry_path) {
                Ok(resolved) => {
                    if !resolved.starts_with(&canonical_root) {
                        // Skip symlinks pointing outside the project root
                        continue;
                    }
                    resolved.is_dir()
                }
                Err(_) => continue,
            }
        } else {
            symlink_meta.is_dir()
        };

        let rel_path = if clean_prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", clean_prefix, name)
        };

        entries.push(ProjectEntry {
            name,
            relative_path: rel_path,
            is_directory: is_dir,
        });

        if entries.len() > 500 {
            return Err(
                "Directory contains more than 500 entries (maximum allowed: 500)".to_string(),
            );
        }
    }

    // Sort: directories first, then alphabetical case-insensitive
    entries.sort_by(|a, b| match (a.is_directory, b.is_directory) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a
            .name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name)),
    });

    Ok(entries)
}

/// Checks if a path is within Yuki's own configuration directory or data root.
pub fn is_yuki_internal_path(config_dir: &Path, canonical_path: &Path) -> bool {
    if let Ok(canon_cfg) = dunce::canonicalize(config_dir) {
        if canonical_path.starts_with(&canon_cfg) {
            return true;
        }
    }
    if let Ok(settings) = load_settings(config_dir) {
        if let Some(ref data_root) = settings.data_root {
            if let Ok(canon_data) = dunce::canonicalize(data_root) {
                if canonical_path.starts_with(&canon_data) {
                    return true;
                }
            }
        }
    }
    false
}

/// Validates an ordinary local file path for reading according to Yuki safety policy.
/// Returns the canonical PathBuf on success.
/// Keeps error messages generic and content-free.
pub fn validate_read_path(config_dir: &Path, path_str: &str) -> Result<PathBuf, String> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return Err("Path cannot be empty".to_string());
    }

    let path = Path::new(trimmed);
    if !path.is_absolute() {
        return Err("Path must be absolute".to_string());
    }

    if is_unc_or_device_path(path) {
        return Err("Device and UNC paths are not permitted".to_string());
    }

    // Check components of raw input path
    for comp in path.components() {
        if let std::path::Component::Normal(name) = comp {
            let s = name.to_string_lossy();
            if is_dos_device_name(&s) {
                return Err("Device and UNC paths are not permitted".to_string());
            }
            if is_blocked_name(&s) {
                return Err("Access to restricted folder or file is blocked".to_string());
            }
        }
    }

    // Reject indirection before resolving it, including Windows junctions.
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor)
            .map_err(|_| "File not found or inaccessible".to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("Links are not permitted for local file reads".to_string());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("Reparse points are not permitted for local file reads".to_string());
            }
        }
    }

    // Canonicalize path and recheck the resolved destination.
    let canonical =
        dunce::canonicalize(path).map_err(|_| "File not found or inaccessible".to_string())?;

    if is_unc_or_device_path(&canonical) {
        return Err("Device and UNC paths are not permitted".to_string());
    }

    if is_drive_root(&canonical) || is_user_profile_root(&canonical) {
        return Err("Access to root directory is blocked".to_string());
    }

    if is_system_directory(&canonical) {
        return Err("Access to system directory is blocked".to_string());
    }

    if is_yuki_internal_path(config_dir, &canonical) {
        return Err("Access to application data or settings is blocked".to_string());
    }

    // Check canonical path components
    for comp in canonical.components() {
        if let std::path::Component::Normal(name) = comp {
            let s = name.to_string_lossy();
            if is_dos_device_name(&s) {
                return Err("Device and UNC paths are not permitted".to_string());
            }
            if is_blocked_name(&s) {
                return Err("Access to restricted folder or file is blocked".to_string());
            }
        }
    }

    // Target must be a regular file
    let meta =
        std::fs::metadata(&canonical).map_err(|_| "Cannot inspect file metadata".to_string())?;
    if !meta.file_type().is_file() {
        return Err("Target is not a regular file".to_string());
    }

    Ok(canonical)
}

/// Lists approved project roots that currently have an active provider-sharing grant.
/// Only project roots with explicit sharing enabled are returned.
pub fn get_shareable_project_roots_impl(config_dir: &Path) -> Result<Vec<String>, String> {
    let settings = load_settings(config_dir)?;
    let mut shareable = Vec::new();

    for root in &settings.project_roots {
        if let Ok(canon_root) = dunce::canonicalize(root) {
            let is_granted = settings.provider_sharing_grants.iter().any(|g| {
                g == root
                    || dunce::canonicalize(g)
                        .map(|cg| cg == canon_root)
                        .unwrap_or(false)
            });
            if is_granted {
                shareable.push(root.clone());
            }
        }
    }

    Ok(shareable)
}

/// Lists immediate entries in an approved project directory, requiring an active provider-sharing grant.
/// Rejects ungranted project roots to prevent leaking directory structure or file metadata to providers.
pub fn list_provider_project_directory_impl(
    config_dir: &Path,
    root: &str,
    relative_path: &str,
) -> Result<Vec<ProjectEntry>, String> {
    let settings = load_settings(config_dir)?;

    let canonical_root = dunce::canonicalize(root)
        .map_err(|e| format!("Failed to canonicalize root directory: {e}"))?;

    let is_approved = settings.project_roots.iter().any(|approved| {
        approved == root
            || dunce::canonicalize(approved)
                .map(|c| c == canonical_root)
                .unwrap_or(false)
    });

    if !is_approved {
        return Err("Root directory is not an approved project folder".to_string());
    }

    let is_granted = settings.provider_sharing_grants.iter().any(|g| {
        g == root
            || dunce::canonicalize(g)
                .map(|cg| cg == canonical_root)
                .unwrap_or(false)
    });

    if !is_granted {
        return Err("Provider sharing is not enabled for this project folder".to_string());
    }

    list_project_directory_impl(config_dir, root, relative_path)
}
