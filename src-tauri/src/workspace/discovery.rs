use std::collections::{HashSet, VecDeque};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::explorer::{
    get_shareable_project_roots_impl, is_blocked_name, is_dos_device_name, is_drive_root,
    is_secret_directory, is_system_directory, is_unc_or_device_path, is_user_profile_root,
    is_yuki_internal_path,
};

pub const DEFAULT_MAX_DEPTH: usize = 4;
pub const MAX_ALLOWED_DEPTH: usize = 8;
pub const MAX_DIRECTORIES_SCANNED: usize = 1000;
pub const MAX_DISCOVERED_PROJECTS: usize = 50;
pub const MAX_SCAN_DURATION: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredProject {
    pub name: String,
    pub engine: String,
    pub path: String,
    pub relative_path: String,
    pub root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryResult {
    pub projects: Vec<DiscoveredProject>,
    pub scanned_roots: Vec<String>,
    pub coverage: String,
    pub truncated: bool,
    pub limits_reached: Vec<String>,
    pub directories_scanned: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscoverGameProjectsArgs {
    #[serde(default)]
    project_root: Option<String>,
    #[serde(default)]
    max_depth: Option<usize>,
}

/// Checks if a folder name matches common build, cache, or dependency directories that should be skipped.
pub fn is_cache_or_generated_dir(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        // Unity
        "library"
            | "temp"
            | "obj"
            | "build"
            | "builds"
            | "logs"
            | "memorycaptures"
            // Godot
            | ".godot"
            | ".import"
            // Unreal
            | "deriveddatacache"
            | "intermediate"
            | "binaries"
            | "saved"
            // General build / dependency / IDE
            | "target"
            | "bin"
            | "node_modules"
            | "dist"
            | "out"
            | ".vs"
            | ".idea"
            | ".vscode"
            | ".gradle"
    )
}

/// Detects if a directory or file is a symlink or reparse point (Windows junction).
pub fn is_symlink_or_reparse(path: &Path) -> bool {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return true;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return true;
            }
        }
    }
    false
}

/// Reads a small bounded text slice (up to 4096 bytes) safely for engine metadata inspection.
fn read_small_file_safely(path: &Path) -> Option<String> {
    if is_symlink_or_reparse(path) {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let mut buffer = [0u8; 4096];
    let bytes_read = file.read(&mut buffer).ok()?;
    if bytes_read == 0 || buffer[..bytes_read].contains(&0) {
        return None;
    }
    std::str::from_utf8(&buffer[..bytes_read])
        .ok()
        .map(|s| s.to_string())
}

/// Extracts Unity editor version from ProjectSettings/ProjectVersion.txt content.
fn extract_unity_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("m_EditorVersion:") {
            let v = rest.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Extracts Godot project name and version from project.godot content.
fn extract_godot_meta(text: &str) -> (Option<String>, Option<String>) {
    let mut name = None;
    let mut version = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if name.is_none() {
            if let Some(rest) = trimmed.strip_prefix("config/name=\"") {
                if let Some(end) = rest.find('"') {
                    name = Some(rest[..end].to_string());
                }
            } else if let Some(rest) = trimmed.strip_prefix("name=\"") {
                if let Some(end) = rest.find('"') {
                    name = Some(rest[..end].to_string());
                }
            }
        }
        if version.is_none() {
            if let Some(rest) = trimmed.strip_prefix("config_version=") {
                let v = rest.trim();
                if v == "5" {
                    version = Some("4.x".to_string());
                } else if v == "4" {
                    version = Some("3.x".to_string());
                }
            }
        }
    }

    (name, version)
}

/// Extracts Unreal engine association from .uproject JSON content.
fn extract_unreal_version(text: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        if let Some(ea) = v.get("EngineAssociation").and_then(|s| s.as_str()) {
            return Some(ea.to_string());
        }
    }
    None
}

/// Inspects a directory to determine if it is a game project (Unity, Godot, or Unreal).
/// Returns DiscoveredProject if an engine marker signature is found.
pub fn identify_game_project(
    dir: &Path,
    canon_root: &Path,
    root_str: &str,
) -> Option<DiscoveredProject> {
    let dir_name = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());

    let rel_path_buf = dir.strip_prefix(canon_root).unwrap_or(Path::new(""));
    let rel_path = rel_path_buf.to_string_lossy().replace('\\', "/");
    let display_rel_path = if rel_path.is_empty() {
        ".".to_string()
    } else {
        rel_path
    };

    // 1. Unity check: ProjectSettings/ProjectVersion.txt (file) + Assets (directory)
    let project_settings = dir.join("ProjectSettings");
    let project_version = project_settings.join("ProjectVersion.txt");
    let assets = dir.join("Assets");

    if project_version.is_file() && assets.is_dir() {
        if !is_symlink_or_reparse(&project_settings)
            && !is_symlink_or_reparse(&project_version)
            && !is_symlink_or_reparse(&assets)
        {
            let version =
                read_small_file_safely(&project_version).and_then(|t| extract_unity_version(&t));
            return Some(DiscoveredProject {
                name: dir_name,
                engine: "Unity".to_string(),
                path: dir.to_string_lossy().to_string(),
                relative_path: display_rel_path,
                root: root_str.to_string(),
                version,
                evidence: vec![
                    "ProjectSettings/ProjectVersion.txt".to_string(),
                    "Assets".to_string(),
                ],
            });
        }
    }

    // 2. Unreal check: *.uproject file directly in directory
    if let Ok(read_dir) = std::fs::read_dir(dir) {
        for entry_res in read_dir {
            if let Ok(entry) = entry_res {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if file_name.to_lowercase().ends_with(".uproject") {
                    let file_path = entry.path();
                    if file_path.is_file() && !is_symlink_or_reparse(&file_path) {
                        let project_name = file_path
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| dir_name.clone());
                        let version = read_small_file_safely(&file_path)
                            .and_then(|t| extract_unreal_version(&t));
                        return Some(DiscoveredProject {
                            name: project_name,
                            engine: "Unreal".to_string(),
                            path: dir.to_string_lossy().to_string(),
                            relative_path: display_rel_path,
                            root: root_str.to_string(),
                            version,
                            evidence: vec![file_name],
                        });
                    }
                }
            }
        }
    }

    // 3. Godot check: project.godot file directly in directory
    let godot_file = dir.join("project.godot");
    if godot_file.is_file() && !is_symlink_or_reparse(&godot_file) {
        let (extracted_name, version) = read_small_file_safely(&godot_file)
            .map(|t| extract_godot_meta(&t))
            .unwrap_or((None, None));
        let project_name = extracted_name.unwrap_or(dir_name);
        return Some(DiscoveredProject {
            name: project_name,
            engine: "Godot".to_string(),
            path: dir.to_string_lossy().to_string(),
            relative_path: display_rel_path,
            root: root_str.to_string(),
            version,
            evidence: vec!["project.godot".to_string()],
        });
    }

    None
}

/// Executes bounded, deterministic discovery for game projects across share-enabled approved roots.
pub fn discover_game_projects_impl(
    config_dir: &Path,
    arguments_json: &str,
) -> Result<DiscoveryResult, String> {
    let args: DiscoverGameProjectsArgs = serde_json::from_str(arguments_json).map_err(|e| {
        format!("Invalid arguments for discover_game_projects: malformed JSON ({e})")
    })?;

    let max_depth = args
        .max_depth
        .unwrap_or(DEFAULT_MAX_DEPTH)
        .clamp(1, MAX_ALLOWED_DEPTH);

    let shareable_roots = get_shareable_project_roots_impl(config_dir)?;

    let target_roots: Vec<String> = if let Some(ref specific_root) = args.project_root {
        let trimmed = specific_root.trim();
        if trimmed.is_empty() {
            return Err("project_root cannot be empty when specified.".to_string());
        }

        let mut matched = None;
        for sr in &shareable_roots {
            if sr == trimmed {
                matched = Some(sr.clone());
                break;
            }
            if let (Ok(c1), Ok(c2)) = (dunce::canonicalize(sr), dunce::canonicalize(trimmed)) {
                if c1 == c2 {
                    matched = Some(sr.clone());
                    break;
                }
            }
        }

        match matched {
            Some(r) => vec![r],
            None => {
                return Err(
                    "Access denied: Root directory is not an approved project folder or provider sharing is not enabled."
                        .to_string(),
                );
            }
        }
    } else {
        shareable_roots
    };

    if target_roots.is_empty() {
        return Ok(DiscoveryResult {
            projects: Vec::new(),
            scanned_roots: Vec::new(),
            coverage: "complete".to_string(),
            truncated: false,
            limits_reached: Vec::new(),
            directories_scanned: 0,
        });
    }

    let start_time = Instant::now();
    let mut discovered_projects = Vec::new();
    let mut scanned_roots = Vec::new();
    let mut limits_reached = HashSet::new();
    let mut total_directories_scanned = 0usize;
    let mut entries_inspected = 0usize;

    for root_str in target_roots {
        let canon_root = match dunce::canonicalize(&root_str) {
            Ok(c) => c,
            Err(_) => {
                limits_reached.insert("inaccessible_root".to_string());
                continue;
            }
        };

        if is_unc_or_device_path(&canon_root)
            || is_drive_root(&canon_root)
            || is_user_profile_root(&canon_root)
            || is_system_directory(&canon_root)
            || is_yuki_internal_path(config_dir, &canon_root)
        {
            continue;
        }

        scanned_roots.push(root_str.clone());

        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();

        visited.insert(canon_root.clone());
        queue.push_back((canon_root.clone(), 0usize));

        let mut hit_stop_limit = false;

        while let Some((curr_dir, curr_depth)) = queue.pop_front() {
            if start_time.elapsed() >= MAX_SCAN_DURATION {
                limits_reached.insert("time_limit".to_string());
                hit_stop_limit = true;
                break;
            }

            if total_directories_scanned >= MAX_DIRECTORIES_SCANNED {
                limits_reached.insert("max_directories".to_string());
                hit_stop_limit = true;
                break;
            }

            total_directories_scanned += 1;

            // Check if curr_dir is a game project
            if let Some(project) = identify_game_project(&curr_dir, &canon_root, &root_str) {
                discovered_projects.push(project);
                if discovered_projects.len() >= MAX_DISCOVERED_PROJECTS {
                    limits_reached.insert("max_results".to_string());
                    hit_stop_limit = true;
                    break;
                }
                // Do not recurse into child directories of an identified game project
                continue;
            }

            // If not a game project, check depth before reading children
            if curr_depth >= max_depth {
                // Check if there were any child directories that could not be traversed
                if let Ok(read_dir) = std::fs::read_dir(&curr_dir) {
                    for entry_res in read_dir {
                        entries_inspected += 1;
                        if entries_inspected > 20_000 || start_time.elapsed() >= MAX_SCAN_DURATION {
                            limits_reached.insert("enumeration_limit".to_string());
                            break;
                        }
                        if let Ok(entry) = entry_res {
                            let name = entry.file_name().to_string_lossy().to_string();
                            if !is_blocked_name(&name)
                                && !is_secret_directory(&name)
                                && !is_cache_or_generated_dir(&name)
                                && !is_dos_device_name(&name)
                            {
                                if let Ok(meta) = entry.metadata() {
                                    if meta.is_dir() {
                                        limits_reached.insert("max_depth".to_string());
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                continue;
            }

            // Read child directories deterministically
            let read_dir = match std::fs::read_dir(&curr_dir) {
                Ok(rd) => rd,
                Err(_) => {
                    limits_reached.insert("inaccessible_directory".to_string());
                    continue;
                }
            };

            let mut child_dirs = Vec::new();
            for entry_res in read_dir {
                entries_inspected += 1;
                if entries_inspected > 20_000 || start_time.elapsed() >= MAX_SCAN_DURATION {
                    limits_reached.insert("enumeration_limit".to_string());
                    hit_stop_limit = true;
                    break;
                }
                let entry = match entry_res {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                let file_name = entry.file_name();
                let name = file_name.to_string_lossy().to_string();

                if is_dos_device_name(&name)
                    || is_blocked_name(&name)
                    || is_secret_directory(&name)
                    || is_cache_or_generated_dir(&name)
                {
                    continue;
                }

                let entry_path = entry.path();
                if is_symlink_or_reparse(&entry_path) {
                    continue;
                }

                let canon_child = match dunce::canonicalize(&entry_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                if !canon_child.is_dir() {
                    continue;
                }

                if !canon_child.starts_with(&canon_root) {
                    continue;
                }

                if is_system_directory(&canon_child)
                    || is_yuki_internal_path(config_dir, &canon_child)
                {
                    continue;
                }

                if visited.contains(&canon_child) {
                    continue;
                }

                child_dirs.push((name, canon_child));
            }

            // Sort deterministically (case-insensitive)
            child_dirs.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

            for (_, child_path) in child_dirs {
                visited.insert(child_path.clone());
                queue.push_back((child_path, curr_depth + 1));
            }
        }

        if hit_stop_limit {
            break;
        }
    }

    let mut limits_vec: Vec<String> = limits_reached.into_iter().collect();
    limits_vec.sort();

    let truncated = !limits_vec.is_empty();
    let coverage = if truncated {
        "partial".to_string()
    } else {
        "complete".to_string()
    };

    Ok(DiscoveryResult {
        projects: discovered_projects,
        scanned_roots,
        coverage,
        truncated,
        limits_reached: limits_vec,
        directories_scanned: total_directories_scanned,
    })
}

/// Reauthorizes discovery results before transmission to ensure no revoked roots or unapproved paths leak.
pub fn reauthorize_discovery_result(config_dir: &Path, result_content: &mut String) {
    let mut parsed: DiscoveryResult = match serde_json::from_str(result_content) {
        Ok(res) => res,
        Err(_) => return,
    };

    let shareable_roots = match get_shareable_project_roots_impl(config_dir) {
        Ok(roots) => roots,
        Err(_) => {
            *result_content = serde_json::to_string(&serde_json::json!({
                "error": "Provider sharing authorization failed before sending to provider: Unable to read shareable roots.",
                "projects": [],
                "scanned_roots": [],
                "coverage": "partial",
                "truncated": true,
                "limits_reached": ["revoked_grant"],
                "directories_scanned": 0
            }))
            .unwrap();
            return;
        }
    };

    if shareable_roots.is_empty() {
        *result_content = serde_json::to_string(&serde_json::json!({
            "error": "Provider sharing authorization failed before sending to provider: Provider sharing was revoked for all scanned project roots.",
            "projects": [],
            "scanned_roots": [],
            "coverage": "partial",
            "truncated": true,
            "limits_reached": ["revoked_grant"],
            "directories_scanned": 0
        }))
        .unwrap();
        return;
    }

    let mut canonical_shareable = Vec::new();
    for sr in &shareable_roots {
        if let Ok(c) = dunce::canonicalize(sr) {
            canonical_shareable.push((sr.clone(), c));
        }
    }

    let original_project_count = parsed.projects.len();
    parsed.projects.retain(|p| {
        let p_path = Path::new(&p.path);
        let Ok(p_canon) = dunce::canonicalize(p_path) else {
            return false;
        };

        canonical_shareable.iter().any(|(sr, sc)| {
            (p.root == *sr
                || dunce::canonicalize(&p.root)
                    .map(|c| c == *sc)
                    .unwrap_or(false))
                && p_canon.starts_with(sc)
        })
    });

    parsed.scanned_roots.retain(|r| {
        canonical_shareable
            .iter()
            .any(|(sr, sc)| r == sr || dunce::canonicalize(r).map(|c| c == *sc).unwrap_or(false))
    });

    if parsed.projects.len() < original_project_count {
        parsed.coverage = "partial".to_string();
        parsed.truncated = true;
        if !parsed.limits_reached.contains(&"revoked_grant".to_string()) {
            parsed.limits_reached.push("revoked_grant".to_string());
            parsed.limits_reached.sort();
        }
    }

    if parsed.scanned_roots.is_empty() {
        *result_content = serde_json::to_string(&serde_json::json!({
            "error": "Provider sharing authorization failed before sending to provider: Provider sharing was revoked for all scanned project roots.",
            "projects": [],
            "scanned_roots": [],
            "coverage": "partial",
            "truncated": true,
            "limits_reached": ["revoked_grant"],
            "directories_scanned": 0
        }))
        .unwrap();
        return;
    }

    *result_content = serde_json::to_string(&parsed).unwrap_or_else(|_| {
        "{\"error\":\"Failed to serialize reauthorized discovery result.\"}".to_string()
    });
}
