use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::discovery::is_symlink_or_reparse;
use super::explorer::{
    is_blocked_name, is_dos_device_name, is_drive_root, is_secret_directory, is_system_directory,
    is_unc_or_device_path, is_user_profile_root, is_yuki_internal_path,
};
use super::models::{
    AgentCommandTemplateGrant, AgentCommandTemplateResult, AgentPermissionSnapshot,
    AgentRevokeResult, AgentWriteRootGrant, AgentWriteRootResult, WorkspaceSettings,
};
use super::settings::{SETTINGS_MUTATION_LOCK, load_settings, save_settings_atomic};

pub const MAX_SCRIPT_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

pub const UNSANDBOXED_DISCLOSURE: &str = "Commands run unsandboxed in host environment. Working directory does not provide containment. Execution is currently disabled in this foundation slice.";

/// Self-contained standard FIPS 180-4 SHA-256 implementation.
/// Requires zero external dependencies.
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h0 = 0x6a09e667u32;
    let mut h1 = 0xbb67ae85u32;
    let mut h2 = 0x3c6ef372u32;
    let mut h3 = 0xa54ff53au32;
    let mut h4 = 0x510e527fu32;
    let mut h5 = 0x9b05688cu32;
    let mut h6 = 0x1f83d9abu32;
    let mut h7 = 0x5be0cd19u32;

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;
        let mut f = h5;
        let mut g = h6;
        let mut h = h7;

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
        h5 = h5.wrapping_add(f);
        h6 = h6.wrapping_add(g);
        h7 = h7.wrapping_add(h);
    }

    format!(
        "{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
        h0, h1, h2, h3, h4, h5, h6, h7
    )
}

/// Validates that a candidate write root is safe, within the approved project root,
/// and complies with all native system, link, and secret exclusions.
pub fn validate_write_root(
    config_dir: &Path,
    project_root_canon: &Path,
    candidate_path_str: &str,
) -> Result<PathBuf, String> {
    let trimmed = candidate_path_str.trim();
    if trimmed.is_empty() {
        return Err("Write root path cannot be empty".to_string());
    }

    let p = Path::new(trimmed);
    if is_unc_or_device_path(p) {
        return Err("Device and UNC paths are not permitted for write roots".to_string());
    }

    if !p.is_absolute() {
        return Err("Write root path must be an absolute path".to_string());
    }

    if !p.exists() {
        return Err(format!(
            "Write root path '{trimmed}' does not exist on disk"
        ));
    }

    // Reject ancestor links as well as the selected folder.
    if p.ancestors().any(is_symlink_or_reparse) {
        return Err(format!(
            "Write root '{trimmed}' is a symlink or reparse point; links and junctions are strictly forbidden"
        ));
    }

    let canonical = dunce::canonicalize(p)
        .map_err(|e| format!("Failed to canonicalize write root '{trimmed}': {e}"))?;

    if !canonical.is_dir() {
        return Err(format!(
            "Write root '{}' is not a directory",
            canonical.display()
        ));
    }

    // Verify canonical path is not a symlink/reparse point
    if is_symlink_or_reparse(&canonical) {
        return Err(format!(
            "Canonical write root '{}' is a symlink or reparse point",
            canonical.display()
        ));
    }

    if is_drive_root(&canonical) {
        return Err("Drive roots cannot be granted as write roots".to_string());
    }

    if is_user_profile_root(&canonical) {
        return Err("User profile roots cannot be granted as write roots".to_string());
    }

    if is_system_directory(&canonical) {
        return Err("System directories cannot be granted as write roots".to_string());
    }

    if is_yuki_internal_path(config_dir, &canonical) {
        return Err("Yuki internal application data cannot be granted as write roots".to_string());
    }

    // Must be contained within the canonical project root
    if !canonical.starts_with(project_root_canon) {
        return Err(format!(
            "Write root '{}' is not contained within approved project root '{}'",
            canonical.display(),
            project_root_canon.display()
        ));
    }

    // Inspect each component between project_root and write_root for secret or system names
    if let Ok(rel) = canonical.strip_prefix(project_root_canon) {
        for comp in rel.components() {
            if let std::path::Component::Normal(name) = comp {
                let s = name.to_string_lossy();
                if is_dos_device_name(&s) {
                    return Err(
                        "Reserved DOS device names are not permitted in write root path"
                            .to_string(),
                    );
                }
                if is_secret_directory(&s) || is_blocked_name(&s) {
                    return Err(format!(
                        "Restricted or secret directory '{s}' cannot be granted as write root"
                    ));
                }
            }
        }
    }

    Ok(canonical)
}

/// Validates command template parameters.
/// Verifies executable, working directory, and script fingerprinting.
pub fn validate_command_template(
    config_dir: &Path,
    project_root_canon: &Path,
    executable: &str,
    args: &[String],
    working_dir: &str,
    script_path: Option<&str>,
) -> Result<(PathBuf, PathBuf, Option<PathBuf>, Option<String>), String> {
    let trimmed_exec = executable.trim();
    if trimmed_exec.is_empty() {
        return Err("Command template executable cannot be empty".to_string());
    }

    let exec_p = Path::new(trimmed_exec);
    if is_unc_or_device_path(exec_p) {
        return Err("Device and UNC paths are not permitted for executables".to_string());
    }
    if !exec_p.is_absolute() {
        return Err("Executable path must be an absolute path".to_string());
    }
    if !exec_p.exists() {
        return Err(format!(
            "Executable '{trimmed_exec}' does not exist on disk"
        ));
    }
    if is_symlink_or_reparse(exec_p) {
        return Err(format!(
            "Executable '{trimmed_exec}' is a symlink or reparse point"
        ));
    }
    let canon_exec = dunce::canonicalize(exec_p)
        .map_err(|e| format!("Failed to canonicalize executable '{trimmed_exec}': {e}"))?;
    if !canon_exec.is_file() {
        return Err(format!(
            "Executable '{}' is not a regular file",
            canon_exec.display()
        ));
    }

    #[cfg(windows)]
    {
        let ext = canon_exec
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let valid_ext = ext.eq_ignore_ascii_case("exe")
            || ext.eq_ignore_ascii_case("cmd")
            || ext.eq_ignore_ascii_case("bat");
        if !valid_ext {
            return Err(format!(
                "Executable '{}' must have an executable extension (.exe, .cmd, .bat)",
                canon_exec.display()
            ));
        }
    }

    if is_yuki_internal_path(config_dir, &canon_exec) {
        return Err("Yuki internal files cannot be registered as command executables".to_string());
    }

    // Working directory validation
    let trimmed_wd = working_dir.trim();
    if trimmed_wd.is_empty() {
        return Err("Command template working directory cannot be empty".to_string());
    }
    let wd_p = Path::new(trimmed_wd);
    if is_unc_or_device_path(wd_p) {
        return Err("Device and UNC paths are not permitted for working directory".to_string());
    }
    if !wd_p.is_absolute() {
        return Err("Working directory must be an absolute path".to_string());
    }
    if !wd_p.exists() {
        return Err(format!(
            "Working directory '{trimmed_wd}' does not exist on disk"
        ));
    }
    if is_symlink_or_reparse(wd_p) {
        return Err(format!(
            "Working directory '{trimmed_wd}' is a symlink or reparse point"
        ));
    }
    let canon_wd = dunce::canonicalize(wd_p)
        .map_err(|e| format!("Failed to canonicalize working directory '{trimmed_wd}': {e}"))?;
    if !canon_wd.is_dir() {
        return Err(format!(
            "Working directory '{}' is not a directory",
            canon_wd.display()
        ));
    }
    if !canon_wd.starts_with(project_root_canon) {
        return Err(format!(
            "Working directory '{}' must be within approved project root '{}'",
            canon_wd.display(),
            project_root_canon.display()
        ));
    }
    if is_system_directory(&canon_wd) || is_yuki_internal_path(config_dir, &canon_wd) {
        return Err("Working directory cannot be in system or application folders".to_string());
    }

    // Script fingerprinting if provided
    let (canon_script, script_hash) = if let Some(sp) = script_path {
        let trimmed_sp = sp.trim();
        if trimmed_sp.is_empty() {
            (None, None)
        } else {
            let sp_p = Path::new(trimmed_sp);
            if is_unc_or_device_path(sp_p) {
                return Err("Device and UNC paths are not permitted for script path".to_string());
            }
            if !sp_p.is_absolute() {
                return Err("Script path must be an absolute path".to_string());
            }
            if !sp_p.exists() {
                return Err(format!("Script file '{trimmed_sp}' does not exist on disk"));
            }
            if is_symlink_or_reparse(sp_p) {
                return Err(format!(
                    "Script file '{trimmed_sp}' is a symlink or reparse point"
                ));
            }
            let canon_sp = dunce::canonicalize(sp_p)
                .map_err(|e| format!("Failed to canonicalize script path '{trimmed_sp}': {e}"))?;
            if !canon_sp.is_file() {
                return Err(format!(
                    "Script path '{}' is not a regular file",
                    canon_sp.display()
                ));
            }
            if !canon_sp.starts_with(project_root_canon) {
                return Err(format!(
                    "Script '{}' must be within approved project root '{}'",
                    canon_sp.display(),
                    project_root_canon.display()
                ));
            }
            if is_system_directory(&canon_sp) || is_yuki_internal_path(config_dir, &canon_sp) {
                return Err("Script cannot be in system or application folders".to_string());
            }

            let meta = std::fs::metadata(&canon_sp)
                .map_err(|e| format!("Failed to inspect script file metadata: {e}"))?;
            if meta.len() > MAX_SCRIPT_BYTES {
                return Err(format!(
                    "Script file size ({} bytes) exceeds maximum allowable review limit ({} bytes)",
                    meta.len(),
                    MAX_SCRIPT_BYTES
                ));
            }

            let mut file = File::open(&canon_sp)
                .map_err(|e| format!("Failed to open script file for fingerprinting: {e}"))?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .map_err(|e| format!("Failed to read script file: {e}"))?;

            let digest = sha256_hex(&buf);
            (Some(canon_sp), Some(digest))
        }
    } else {
        (None, None)
    };

    // Validate args
    for arg in args {
        if arg.contains('\0') {
            return Err("Command arguments cannot contain null bytes".to_string());
        }
    }

    Ok((canon_exec, canon_wd, canon_script, script_hash))
}

/// Derives the effective permission snapshot dynamically from WorkspaceSettings for a specific task root.
/// Does not copy stale grants; checks that paths still exist on disk and project root remains approved.
pub fn derive_permission_snapshot(
    settings: &WorkspaceSettings,
    task_root_opt: Option<&str>,
) -> AgentPermissionSnapshot {
    let generation = settings.grant_generation;

    let (task_root_str, is_approved, canon_task_root) = match task_root_opt {
        Some(tr) if !tr.trim().is_empty() => {
            let canon = dunce::canonicalize(tr.trim()).ok();
            let is_app = settings.project_roots.iter().any(|r| {
                r == tr.trim()
                    || match (dunce::canonicalize(r), canon.as_ref()) {
                        (Ok(rc), Some(tc)) => &rc == tc,
                        _ => false,
                    }
            });
            let resolved_str = canon
                .as_ref()
                .map(|c| c.to_string_lossy().to_string())
                .unwrap_or_else(|| tr.trim().to_string());
            (Some(resolved_str), is_app, canon)
        }
        _ => (None, false, None),
    };

    let provider_sharing_enabled = if is_approved {
        if let Some(ref tc) = canon_task_root {
            settings
                .provider_sharing_grants
                .iter()
                .any(|g| dunce::canonicalize(g).map(|gc| &gc == tc).unwrap_or(false))
        } else {
            false
        }
    } else {
        false
    };

    let write_roots = if is_approved {
        if let Some(ref tc) = canon_task_root {
            settings
                .agent_write_roots
                .iter()
                .filter(|w| {
                    let canon_pr = dunce::canonicalize(&w.project_root).ok();
                    let belongs = canon_pr.as_ref() == Some(tc);
                    if !belongs {
                        return false;
                    }
                    // Validate path still exists on disk, is a directory, and not symlink
                    if let Ok(canon_w) = dunce::canonicalize(&w.canonical_path) {
                        canon_w.is_dir()
                            && canon_w.starts_with(tc)
                            && !is_symlink_or_reparse(&canon_w)
                    } else {
                        false
                    }
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let command_templates = if is_approved {
        if let Some(ref tc) = canon_task_root {
            settings
                .agent_command_templates
                .iter()
                .filter(|t| {
                    let canon_pr = dunce::canonicalize(&t.project_root).ok();
                    let belongs = canon_pr.as_ref() == Some(tc);
                    if !belongs {
                        return false;
                    }
                    let wd_ok = dunce::canonicalize(&t.working_dir_root)
                        .map(|w| w.is_dir() && w.starts_with(tc) && !is_symlink_or_reparse(&w))
                        .unwrap_or(false);
                    let exec_ok = dunce::canonicalize(&t.canonical_executable)
                        .map(|e| e.is_file() && !is_symlink_or_reparse(&e))
                        .unwrap_or(false);
                    wd_ok && exec_ok
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    AgentPermissionSnapshot {
        generation,
        task_root: task_root_str,
        is_task_root_approved: is_approved,
        provider_sharing_enabled,
        write_roots,
        command_templates,
        execution_available: false,
        unsandboxed_disclosure: UNSANDBOXED_DISCLOSURE.to_string(),
    }
}

/// Retrieves the agent permission snapshot for a given project root or conversation ID.
pub fn get_agent_permission_snapshot_impl(
    config_dir: &Path,
    project_root: Option<&str>,
    conversation_id: Option<&str>,
) -> Result<AgentPermissionSnapshot, String> {
    let settings = load_settings(config_dir)?;
    if let Some(cid) = conversation_id {
        crate::chat::history::validate_conversation_id(cid)?;
    }

    let effective_task_root = if let Some(pr) = project_root {
        if !pr.trim().is_empty() {
            Some(pr.trim().to_string())
        } else {
            None
        }
    } else if let Some(cid) = conversation_id {
        if !cid.trim().is_empty() {
            if let Some(ref data_root_str) = settings.data_root {
                let data_root = PathBuf::from(data_root_str);
                if let Ok(conv) =
                    crate::chat::history::load_conversation_from_disk(&data_root, cid.trim())
                {
                    conv.task_root
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    Ok(derive_permission_snapshot(
        &settings,
        effective_task_root.as_deref(),
    ))
}

/// Approves an exact canonical write root within an approved project root.
/// Enforces UUID assignment, monotonic generation advancement, optimistic concurrency checks,
/// and protection under SETTINGS_MUTATION_LOCK.
pub fn approve_agent_write_root_impl(
    config_dir: &Path,
    project_root_str: &str,
    candidate_path_str: &str,
    expected_generation: Option<u64>,
) -> Result<AgentWriteRootResult, String> {
    let _guard = SETTINGS_MUTATION_LOCK
        .lock()
        .map_err(|e| format!("Failed to acquire settings mutation lock: {e}"))?;

    let mut settings = load_settings(config_dir)?;

    let expected =
        expected_generation.ok_or_else(|| "Expected grant generation is required".to_string())?;
    {
        if expected != settings.grant_generation {
            return Err(format!(
                "Stale grant generation: expected {expected}, current {}",
                settings.grant_generation
            ));
        }
    }

    let canon_pr = dunce::canonicalize(project_root_str)
        .map_err(|e| format!("Cannot inspect project root '{project_root_str}': {e}"))?;
    let canon_pr_str = canon_pr.to_string_lossy().to_string();

    let is_approved = settings.project_roots.iter().any(|r| {
        r == project_root_str
            || dunce::canonicalize(r)
                .map(|c| c == canon_pr)
                .unwrap_or(false)
    });
    if !is_approved {
        return Err(format!(
            "Cannot approve write root for unapproved project folder '{project_root_str}'"
        ));
    }

    let canon_wr = validate_write_root(config_dir, &canon_pr, candidate_path_str)?;
    let canon_wr_str = canon_wr.to_string_lossy().to_string();

    let already_present = settings.agent_write_roots.iter().find(|w| {
        w.canonical_path == canon_wr_str
            || dunce::canonicalize(&w.canonical_path)
                .map(|c| c == canon_wr)
                .unwrap_or(false)
    });

    if let Some(existing) = already_present {
        return Err(format!(
            "Write root '{}' is already approved with grant ID '{}'",
            canon_wr_str, existing.id
        ));
    }

    settings.grant_generation = settings.grant_generation.saturating_add(1);
    let grant_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let grant = AgentWriteRootGrant {
        id: grant_id,
        canonical_path: canon_wr_str,
        project_root: canon_pr_str.clone(),
        created_at: now,
        generation: settings.grant_generation,
    };

    settings.agent_write_roots.push(grant.clone());
    save_settings_atomic(config_dir, &settings)?;

    let snapshot = derive_permission_snapshot(&settings, Some(&canon_pr_str));
    Ok(AgentWriteRootResult { grant, snapshot })
}

/// Revokes approval for a write root grant by UUID.
/// Enforces generation checks and returns updated snapshot under SETTINGS_MUTATION_LOCK.
pub fn revoke_agent_write_root_impl(
    config_dir: &Path,
    grant_id: &str,
    project_root_hint: Option<&str>,
    expected_generation: Option<u64>,
) -> Result<AgentRevokeResult, String> {
    let _guard = SETTINGS_MUTATION_LOCK
        .lock()
        .map_err(|e| format!("Failed to acquire settings mutation lock: {e}"))?;

    let mut settings = load_settings(config_dir)?;

    let expected =
        expected_generation.ok_or_else(|| "Expected grant generation is required".to_string())?;
    {
        if expected != settings.grant_generation {
            return Err(format!(
                "Stale grant generation: expected {expected}, current {}",
                settings.grant_generation
            ));
        }
    }

    let idx = settings
        .agent_write_roots
        .iter()
        .position(|w| w.id == grant_id)
        .ok_or_else(|| format!("Grant ID '{grant_id}' not found or already revoked."))?;

    let removed = settings.agent_write_roots.remove(idx);
    settings.grant_generation = settings.grant_generation.saturating_add(1);

    save_settings_atomic(config_dir, &settings)?;

    let target_root = project_root_hint.unwrap_or(&removed.project_root);
    let snapshot = derive_permission_snapshot(&settings, Some(target_root));

    Ok(AgentRevokeResult {
        revoked_id: grant_id.to_string(),
        snapshot,
    })
}

/// Creates a reviewed command template record.
/// Enforces script fingerprinting (if offered) and explicitly marks execution_available: false.
pub fn create_agent_command_template_impl(
    config_dir: &Path,
    project_root_str: &str,
    executable: &str,
    args: Vec<String>,
    working_dir: &str,
    script_path: Option<String>,
    expected_generation: Option<u64>,
) -> Result<AgentCommandTemplateResult, String> {
    let _guard = SETTINGS_MUTATION_LOCK
        .lock()
        .map_err(|e| format!("Failed to acquire settings mutation lock: {e}"))?;

    let mut settings = load_settings(config_dir)?;

    let expected =
        expected_generation.ok_or_else(|| "Expected grant generation is required".to_string())?;
    {
        if expected != settings.grant_generation {
            return Err(format!(
                "Stale grant generation: expected {expected}, current {}",
                settings.grant_generation
            ));
        }
    }

    let canon_pr = dunce::canonicalize(project_root_str)
        .map_err(|e| format!("Cannot inspect project root '{project_root_str}': {e}"))?;
    let canon_pr_str = canon_pr.to_string_lossy().to_string();

    let is_approved = settings.project_roots.iter().any(|r| {
        r == project_root_str
            || dunce::canonicalize(r)
                .map(|c| c == canon_pr)
                .unwrap_or(false)
    });
    if !is_approved {
        return Err(format!(
            "Cannot create command template for unapproved project folder '{project_root_str}'"
        ));
    }

    let (canon_exec, canon_wd, canon_script, script_hash) = validate_command_template(
        config_dir,
        &canon_pr,
        executable,
        &args,
        working_dir,
        script_path.as_deref(),
    )?;

    let canon_exec_str = canon_exec.to_string_lossy().to_string();
    let canon_wd_str = canon_wd.to_string_lossy().to_string();
    let canon_script_str = canon_script.map(|s| s.to_string_lossy().to_string());

    let already_present = settings.agent_command_templates.iter().find(|t| {
        t.project_root == canon_pr_str
            && t.canonical_executable == canon_exec_str
            && t.args == args
            && t.working_dir_root == canon_wd_str
            && t.script_path == canon_script_str
    });

    if let Some(existing) = already_present {
        return Err(format!(
            "Identical command template already registered with grant ID '{}'",
            existing.id
        ));
    }

    settings.grant_generation = settings.grant_generation.saturating_add(1);
    let grant_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let grant = AgentCommandTemplateGrant {
        id: grant_id,
        project_root: canon_pr_str.clone(),
        canonical_executable: canon_exec_str,
        args,
        working_dir_root: canon_wd_str,
        script_path: canon_script_str,
        script_sha256: script_hash,
        created_at: now,
        generation: settings.grant_generation,
        execution_available: false,
    };

    settings.agent_command_templates.push(grant.clone());
    save_settings_atomic(config_dir, &settings)?;

    let snapshot = derive_permission_snapshot(&settings, Some(&canon_pr_str));
    Ok(AgentCommandTemplateResult { grant, snapshot })
}

/// Revokes a command template grant by UUID.
pub fn revoke_agent_command_template_impl(
    config_dir: &Path,
    grant_id: &str,
    project_root_hint: Option<&str>,
    expected_generation: Option<u64>,
) -> Result<AgentRevokeResult, String> {
    let _guard = SETTINGS_MUTATION_LOCK
        .lock()
        .map_err(|e| format!("Failed to acquire settings mutation lock: {e}"))?;

    let mut settings = load_settings(config_dir)?;

    let expected =
        expected_generation.ok_or_else(|| "Expected grant generation is required".to_string())?;
    {
        if expected != settings.grant_generation {
            return Err(format!(
                "Stale grant generation: expected {expected}, current {}",
                settings.grant_generation
            ));
        }
    }

    let idx = settings
        .agent_command_templates
        .iter()
        .position(|t| t.id == grant_id)
        .ok_or_else(|| format!("Grant ID '{grant_id}' not found or already revoked."))?;

    let removed = settings.agent_command_templates.remove(idx);
    settings.grant_generation = settings.grant_generation.saturating_add(1);

    save_settings_atomic(config_dir, &settings)?;

    let target_root = project_root_hint.unwrap_or(&removed.project_root);
    let snapshot = derive_permission_snapshot(&settings, Some(target_root));

    Ok(AgentRevokeResult {
        revoked_id: grant_id.to_string(),
        snapshot,
    })
}

/// Explicit execution stub: execution is intentionally disabled in this foundation slice.
pub fn execute_agent_command_template_stub(_grant_id: &str) -> Result<(), String> {
    Err("Command execution is unavailable in this foundation slice. Command templates are stored as reviewed records only.".to_string())
}
