use std::path::{Path, PathBuf};

use super::discovery::{
    discover_game_projects_impl, identify_game_project, reauthorize_discovery_result,
};
use super::explorer::{
    get_shareable_project_roots_impl, is_blocked_name, is_drive_root, is_system_directory,
    is_user_profile_root, list_project_directory_impl, list_provider_project_directory_impl,
    validate_project_root,
};
use super::models::WorkspaceSettings;
use super::reader::{MAX_CHUNK_SIZE, authorize_provider_file, read_local_file_chunk_impl};
use super::search::{
    FileMatchType, glob_match, matches_file_pattern, reauthorize_search_files_result,
    reauthorize_search_text_result, search_project_files_impl, search_project_text_impl,
};
use super::settings::{
    add_project_root_path, choose_data_root_path, get_workspace_settings_impl, load_settings,
    remove_project_root_impl, resolve_data_root_subdir, save_settings_atomic,
    toggle_project_provider_sharing_impl,
};
use crate::chat::tools::execute_tool;

/// Test helper providing an isolated sandbox within the project directory (target/test_temp),
/// strictly avoiding /tmp, /var/tmp, or user directories.
struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(name: &str) -> Self {
        let unique = format!(
            "{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let base = dunce::canonicalize(std::env::current_dir().unwrap())
            .unwrap()
            .join("target")
            .join("test_temp")
            .join(unique);
        std::fs::create_dir_all(&base).unwrap();
        Self { root: base }
    }

    fn config_dir(&self) -> PathBuf {
        let dir = self.root.join("config");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn new_dir(&self, name: &str) -> PathBuf {
        let dir = self.root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_settings_persistence_and_reload() {
    let env = TestEnv::new("persist_reload");
    let cfg = env.config_dir();

    // Initial load returns default settings
    let initial = get_workspace_settings_impl(&cfg).unwrap();
    assert_eq!(initial.data_root, None);
    assert!(initial.project_roots.is_empty());

    // Save populated settings
    let mut modified = WorkspaceSettings::default();
    modified.data_root = Some("D:\\YukiData\\Yuki".to_string());
    modified.project_roots = vec![
        "D:\\Projects\\App1".to_string(),
        "D:\\Projects\\App2".to_string(),
    ];
    save_settings_atomic(&cfg, &modified).unwrap();

    // Reload from disk and verify
    let reloaded = load_settings(&cfg).unwrap();
    assert_eq!(reloaded, modified);
}

#[test]
fn test_atomic_save_leaves_no_lingering_temp_or_backup() {
    let env = TestEnv::new("atomic_save");
    let cfg = env.config_dir();

    let mut settings = WorkspaceSettings::default();
    settings.data_root = Some("D:\\Yuki".to_string());

    // Save multiple times to test backup creation and removal
    save_settings_atomic(&cfg, &settings).unwrap();
    settings
        .project_roots
        .push("D:\\Projects\\Test".to_string());
    save_settings_atomic(&cfg, &settings).unwrap();

    let dest = cfg.join("settings.json");
    assert!(dest.exists());

    // Verify no .tmp or .bak files remain
    for entry in std::fs::read_dir(&cfg).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name != "settings.json" {
            panic!("Unexpected leftover file in config dir: {}", name);
        }
    }
}

#[test]
fn test_choose_data_root_subdir_resolution_and_no_doubling() {
    let env = TestEnv::new("subdir_resolve");
    let picked_normal = env.new_dir("CustomFolder");
    let resolved = resolve_data_root_subdir(&picked_normal);
    assert_eq!(resolved.file_name().unwrap(), "Yuki");

    let picked_yuki = env.new_dir("Yuki");
    let resolved_yuki = resolve_data_root_subdir(&picked_yuki);
    // Should NOT double to Yuki/Yuki
    assert_eq!(resolved_yuki, picked_yuki);

    let picked_lower = env.new_dir("yuki");
    let resolved_lower = resolve_data_root_subdir(&picked_lower);
    assert_eq!(resolved_lower, picked_lower);
}

#[test]
fn test_choose_data_root_creates_subdir_and_probes_writable() {
    let env = TestEnv::new("data_root_writable");
    let cfg = env.config_dir();
    let picked = env.new_dir("Storage");

    let updated = choose_data_root_path(&cfg, &picked).unwrap();
    assert!(updated.data_root.is_some());

    let canonical_data_root = PathBuf::from(updated.data_root.unwrap());
    assert!(canonical_data_root.exists());
    assert_eq!(canonical_data_root.file_name().unwrap(), "Yuki");

    // The probe file should be deleted after write test
    let mut files = std::fs::read_dir(&canonical_data_root).unwrap();
    assert!(
        files.next().is_none(),
        "Data root should be clean after probe"
    );
}

#[test]
fn test_choose_data_root_reject_when_old_root_nonempty() {
    let env = TestEnv::new("old_root_nonempty");
    let cfg = env.config_dir();
    let initial_picked = env.new_dir("InitialFolder");

    let settings = choose_data_root_path(&cfg, &initial_picked).unwrap();
    let initial_data_root = PathBuf::from(settings.data_root.unwrap());

    // Add a file to initial data root
    std::fs::write(initial_data_root.join("history.db"), b"data").unwrap();

    // Attempt to change data root to another folder
    let second_picked = env.new_dir("SecondFolder");
    let err = choose_data_root_path(&cfg, &second_picked).unwrap_err();
    assert!(
        err.contains("not empty"),
        "Expected error message indicating current data folder is not empty, got: {err}"
    );

    // Verify settings still point to initial data root
    let current = load_settings(&cfg).unwrap();
    assert_eq!(
        current.data_root.as_deref(),
        Some(initial_data_root.to_str().unwrap())
    );
}

#[test]
fn test_choose_data_root_allows_change_when_old_root_empty() {
    let env = TestEnv::new("old_root_empty");
    let cfg = env.config_dir();
    let first = env.new_dir("FirstFolder");
    choose_data_root_path(&cfg, &first).unwrap();

    let second = env.new_dir("SecondFolder");
    let updated = choose_data_root_path(&cfg, &second).unwrap();
    let target = dunce::canonicalize(second.join("Yuki")).unwrap();
    assert_eq!(updated.data_root.as_deref(), Some(target.to_str().unwrap()));
}

#[test]
fn test_choose_data_root_same_path_noop() {
    let env = TestEnv::new("same_path_noop");
    let cfg = env.config_dir();
    let folder = env.new_dir("SameFolder");

    choose_data_root_path(&cfg, &folder).unwrap();

    // Place an entry in the data root
    let data_root = folder.join("Yuki");
    std::fs::write(data_root.join("notes.txt"), b"important").unwrap();

    // Calling choose_data_root_path with the exact same picked folder should be a no-op and succeed
    let result = choose_data_root_path(&cfg, &folder);
    assert!(result.is_ok(), "Selecting same path should be a no-op");
}

#[test]
fn test_add_project_root_excludes_system_and_drive_roots() {
    // Whole drive root check
    #[cfg(windows)]
    {
        assert!(is_drive_root(Path::new("C:\\")));
        assert!(is_drive_root(Path::new("D:\\")));
    }
    #[cfg(unix)]
    {
        assert!(is_drive_root(Path::new("/")));
    }
    assert!(!is_drive_root(Path::new("C:\\Users\\John\\Projects")));

    // User profile root check
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home_path = dunce::canonicalize(&home).unwrap();
        assert!(is_user_profile_root(&home_path));
    }
    #[cfg(windows)]
    {
        assert!(is_user_profile_root(Path::new("C:\\Users")));
    }

    // Windows system dir check
    #[cfg(windows)]
    {
        assert!(validate_project_root(Path::new("C:\\")).is_err());
        if let Some(windir) = std::env::var_os("windir") {
            let win_path = dunce::canonicalize(&windir).unwrap();
            assert!(is_system_directory(&win_path));
            assert!(is_system_directory(&win_path.join("System32")));
            assert!(validate_project_root(&win_path).is_err());
        }
    }
}

#[test]
fn test_add_project_root_validates_and_persists() {
    let env = TestEnv::new("add_remove_project");
    let cfg = env.config_dir();
    let project = env.new_dir("MyValidProject");

    // Add project root
    let settings = add_project_root_path(&cfg, &project).unwrap();
    let canon_project = dunce::canonicalize(&project)
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert_eq!(settings.project_roots, vec![canon_project.clone()]);

    // Adding duplicate project root is a no-op
    let settings2 = add_project_root_path(&cfg, &project).unwrap();
    assert_eq!(settings2.project_roots.len(), 1);

    // Create a file in the project folder
    let file_path = project.join("src_file.rs");
    std::fs::write(&file_path, b"fn main() {}").unwrap();

    // Remove project root: revokes only, preserves files
    let settings3 = remove_project_root_impl(&cfg, &canon_project).unwrap();
    assert!(settings3.project_roots.is_empty());

    // File on disk must remain untouched
    assert!(file_path.exists());
}

#[test]
fn test_list_project_directory_unapproved_root_rejected() {
    let env = TestEnv::new("unapproved_root");
    let cfg = env.config_dir();
    let unapproved = env.new_dir("Unapproved");

    let err = list_project_directory_impl(&cfg, unapproved.to_str().unwrap(), "").unwrap_err();
    assert!(
        err.contains("not an approved project folder"),
        "Expected unapproved root error, got: {err}"
    );
}

#[test]
fn test_list_project_directory_traversal_rejected() {
    let env = TestEnv::new("traversal_rejected");
    let cfg = env.config_dir();
    let project = env.new_dir("ProjectDir");
    add_project_root_path(&cfg, &project).unwrap();
    let root_str = project.to_str().unwrap();

    assert!(list_project_directory_impl(&cfg, root_str, "..").is_err());
    assert!(list_project_directory_impl(&cfg, root_str, "../other").is_err());
    assert!(list_project_directory_impl(&cfg, root_str, "sub/../../escape").is_err());
}

#[test]
fn test_list_project_directory_secret_exclusions() {
    let env = TestEnv::new("secret_exclusions");
    let cfg = env.config_dir();
    let project = env.new_dir("SecProject");
    add_project_root_path(&cfg, &project).unwrap();
    let root_str = project.to_str().unwrap();

    // Create allowed files and directories
    let src_dir = project.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("main.rs"), b"code").unwrap();
    std::fs::write(project.join("README.md"), b"# Readme").unwrap();

    // Create secret files that must be excluded
    std::fs::write(project.join(".env"), b"SECRET=1").unwrap();
    std::fs::write(project.join(".env.local"), b"SECRET=2").unwrap();
    std::fs::write(project.join("credentials.json"), b"{}").unwrap();
    std::fs::write(project.join("id_rsa"), b"key").unwrap();
    std::fs::write(project.join("server.key"), b"key").unwrap();
    std::fs::write(project.join("cert.pem"), b"cert").unwrap();

    // Create .git directory
    let git_dir = project.join(".git");
    std::fs::create_dir_all(&git_dir).unwrap();
    std::fs::write(git_dir.join("HEAD"), b"ref").unwrap();

    // Verify is_blocked_name directly
    assert!(is_blocked_name(".env"));
    assert!(is_blocked_name(".env.production"));
    assert!(is_blocked_name("credentials"));
    assert!(is_blocked_name("credentials.txt"));
    assert!(is_blocked_name("private.key"));
    assert!(is_blocked_name("certificate.pem"));
    assert!(is_blocked_name("id_rsa"));
    assert!(is_blocked_name("id_ed25519"));
    assert!(is_blocked_name(".git"));
    assert!(!is_blocked_name("README.md"));
    assert!(!is_blocked_name("src"));

    // List root directory
    let entries = list_project_directory_impl(&cfg, root_str, "").unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();

    // Only src and README.md should be visible
    assert_eq!(names, vec!["src", "README.md"]);

    // Attempting to directly list .git should be blocked
    let git_err = list_project_directory_impl(&cfg, root_str, ".git").unwrap_err();
    assert!(
        git_err.contains("blocked"),
        "Expected blocked folder error, got: {git_err}"
    );
}

#[test]
fn test_list_project_directory_sorting_and_relative_path() {
    let env = TestEnv::new("dir_sorting");
    let cfg = env.config_dir();
    let project = env.new_dir("SortedProject");
    add_project_root_path(&cfg, &project).unwrap();
    let root_str = project.to_str().unwrap();

    std::fs::create_dir_all(project.join("zoo_dir")).unwrap();
    std::fs::create_dir_all(project.join("alpha_dir")).unwrap();
    std::fs::write(project.join("zoo_file.txt"), b"").unwrap();
    std::fs::write(project.join("alpha_file.txt"), b"").unwrap();

    let entries = list_project_directory_impl(&cfg, root_str, "").unwrap();

    // Directories first, then files, alphabetical case-insensitive
    assert_eq!(entries[0].name, "alpha_dir");
    assert!(entries[0].is_directory);
    assert_eq!(entries[0].relative_path, "alpha_dir");

    assert_eq!(entries[1].name, "zoo_dir");
    assert!(entries[1].is_directory);
    assert_eq!(entries[1].relative_path, "zoo_dir");

    assert_eq!(entries[2].name, "alpha_file.txt");
    assert!(!entries[2].is_directory);
    assert_eq!(entries[2].relative_path, "alpha_file.txt");

    assert_eq!(entries[3].name, "zoo_file.txt");
    assert!(!entries[3].is_directory);
    assert_eq!(entries[3].relative_path, "zoo_file.txt");
}

#[test]
fn test_list_project_directory_cap_500_entries() {
    let env = TestEnv::new("cap_500");
    let cfg = env.config_dir();
    let project = env.new_dir("BigProject");
    add_project_root_path(&cfg, &project).unwrap();
    let root_str = project.to_str().unwrap();

    let sub = project.join("many_files");
    std::fs::create_dir_all(&sub).unwrap();

    for i in 0..505 {
        std::fs::write(sub.join(format!("file_{:03}.txt", i)), b"").unwrap();
    }

    let err = list_project_directory_impl(&cfg, root_str, "many_files").unwrap_err();
    assert!(
        err.contains("500"),
        "Expected error citing 500 entry limit, got: {err}"
    );
}

#[test]
fn test_external_symlink_or_junction_rejected() {
    let env = TestEnv::new("symlink_test");
    let cfg = env.config_dir();
    let project = env.new_dir("ProjWithLink");
    let outside = env.new_dir("OutsideSecretDir");
    std::fs::write(outside.join("secret_data.txt"), b"confidential").unwrap();

    add_project_root_path(&cfg, &project).unwrap();
    let root_str = project.to_str().unwrap();

    let link_path = project.join("external_link");

    // Attempt to create symlink (or directory junction on Windows)
    #[cfg(windows)]
    let link_created = std::os::windows::fs::symlink_dir(&outside, &link_path).is_ok();
    #[cfg(unix)]
    let link_created = std::os::unix::fs::symlink(&outside, &link_path).is_ok();

    if link_created {
        // 1. Listing the root should not expose the external link
        let entries = list_project_directory_impl(&cfg, root_str, "").unwrap();
        assert!(
            entries.iter().all(|e| e.name != "external_link"),
            "External symlink should not be listed as valid entry"
        );

        // 2. Navigating into the external symlink must fail with escape error
        let err = list_project_directory_impl(&cfg, root_str, "external_link");
        assert!(err.is_err(), "Navigating into external symlink must fail");
    }
}

#[test]
fn test_read_local_file_chunk_success() {
    let env = TestEnv::new("read_chunk_success");
    let cfg = env.config_dir();
    let dir = env.new_dir("project");
    let file_path = dir.join("hello.txt");
    let content = "Hello Yuki! This is an ordinary text file.";
    std::fs::write(&file_path, content.as_bytes()).unwrap();

    let canon = dunce::canonicalize(&file_path).unwrap();
    let chunk = read_local_file_chunk_impl(&cfg, canon.to_str().unwrap(), 0, None).unwrap();

    assert_eq!(chunk.path, canon.to_string_lossy().to_string());
    assert_eq!(chunk.text, content);
    assert_eq!(chunk.offset, 0);
    assert_eq!(chunk.next_offset, content.len() as u64);
    assert_eq!(chunk.total_bytes, content.len() as u64);
    assert!(chunk.eof);

    // Reading at EOF returns empty chunk with eof=true
    let eof_chunk =
        read_local_file_chunk_impl(&cfg, canon.to_str().unwrap(), chunk.next_offset, None).unwrap();
    assert_eq!(eof_chunk.text, "");
    assert_eq!(eof_chunk.offset, chunk.next_offset);
    assert_eq!(eof_chunk.next_offset, chunk.next_offset);
    assert!(eof_chunk.eof);
}

#[test]
fn test_read_local_file_chunk_range() {
    let env = TestEnv::new("read_chunk_range");
    let cfg = env.config_dir();
    let dir = env.new_dir("project");
    let file_path = dir.join("range.txt");
    let content = "0123456789abcdefghijklmnopqrstuvwxyz";
    std::fs::write(&file_path, content.as_bytes()).unwrap();

    let canon = dunce::canonicalize(&file_path).unwrap();
    let chunk1 = read_local_file_chunk_impl(&cfg, canon.to_str().unwrap(), 10, Some(10)).unwrap();
    assert_eq!(chunk1.text, "abcdefghij");
    assert_eq!(chunk1.offset, 10);
    assert_eq!(chunk1.next_offset, 20);
    assert_eq!(chunk1.total_bytes, content.len() as u64);
    assert!(!chunk1.eof);

    let chunk2 =
        read_local_file_chunk_impl(&cfg, canon.to_str().unwrap(), chunk1.next_offset, Some(20))
            .unwrap();
    assert_eq!(chunk2.text, "klmnopqrstuvwxyz");
    assert_eq!(chunk2.offset, 20);
    assert_eq!(chunk2.next_offset, content.len() as u64);
    assert!(chunk2.eof);
}

#[test]
fn test_read_local_file_chunk_large_file() {
    let env = TestEnv::new("read_chunk_large");
    let cfg = env.config_dir();
    let dir = env.new_dir("project");
    let file_path = dir.join("large.txt");

    // Create 150 KiB file (153,600 bytes)
    let pattern = "0123456789ABCDEF"; // 16 bytes
    let repeats = 150 * 1024 / 16;
    let mut large_content = String::with_capacity(repeats * 16);
    for _ in 0..repeats {
        large_content.push_str(pattern);
    }
    std::fs::write(&file_path, large_content.as_bytes()).unwrap();

    let canon = dunce::canonicalize(&file_path).unwrap();
    let path_str = canon.to_str().unwrap();

    let mut reconstructed = String::new();
    let mut offset = 0u64;
    let mut chunks_read = 0;

    loop {
        let chunk = read_local_file_chunk_impl(&cfg, path_str, offset, None).unwrap();
        reconstructed.push_str(&chunk.text);
        assert_eq!(chunk.total_bytes, large_content.len() as u64);
        chunks_read += 1;
        if chunk.eof {
            assert_eq!(chunk.next_offset, large_content.len() as u64);
            break;
        }
        assert_eq!(chunk.next_offset, offset + MAX_CHUNK_SIZE as u64);
        offset = chunk.next_offset;
    }

    assert_eq!(chunks_read, 3); // 64 KiB + 64 KiB + 22 KiB = 3 chunks
    assert_eq!(reconstructed.len(), large_content.len());
    assert_eq!(reconstructed, large_content);
}

#[test]
fn test_read_local_file_chunk_multibyte_utf8_boundaries() {
    let env = TestEnv::new("read_chunk_multibyte");
    let cfg = env.config_dir();
    let dir = env.new_dir("project");
    let file_path = dir.join("multibyte.txt");

    // "Yuki - 雪 - ❄️"
    // '雪' is 3 bytes (offsets 7..10).
    let content = "Yuki - 雪 - ❄️";
    std::fs::write(&file_path, content.as_bytes()).unwrap();

    let canon = dunce::canonicalize(&file_path).unwrap();
    let path_str = canon.to_str().unwrap();

    // Request chunk size 8: lands on first byte of '雪' (offset 7)
    let chunk1 = read_local_file_chunk_impl(&cfg, path_str, 0, Some(8)).unwrap();
    // Must roll back to character boundary: "Yuki - " (7 bytes)
    assert_eq!(chunk1.text, "Yuki - ");
    assert_eq!(chunk1.offset, 0);
    assert_eq!(chunk1.next_offset, 7);
    assert!(!chunk1.eof);

    // Next chunk starting at offset 7
    let chunk2 = read_local_file_chunk_impl(&cfg, path_str, chunk1.next_offset, Some(8)).unwrap();
    // Next 8 bytes: '雪' (3) + " - " (3) + 2 bytes of '❄' -> rolls back to "雪 - " (6 bytes)
    assert_eq!(chunk2.text, "雪 - ");
    assert_eq!(chunk2.offset, 7);
    assert_eq!(chunk2.next_offset, 13);
    assert!(!chunk2.eof);

    // Final chunk starting at offset 13 reads '❄️' to EOF
    let chunk3 = read_local_file_chunk_impl(&cfg, path_str, chunk2.next_offset, Some(8)).unwrap();
    assert_eq!(chunk3.text, "❄️");
    assert_eq!(chunk3.offset, 13);
    assert!(chunk3.eof);

    let full = format!("{}{}{}", chunk1.text, chunk2.text, chunk3.text);
    assert_eq!(full, content);

    // Also test single-character completion when chunk_size is 1 byte on a 3-byte char
    let single_char_chunk = read_local_file_chunk_impl(&cfg, path_str, 7, Some(1)).unwrap();
    assert_eq!(single_char_chunk.text, "雪");
    assert_eq!(single_char_chunk.offset, 7);
    assert_eq!(single_char_chunk.next_offset, 10);
}

#[test]
fn test_read_local_file_chunk_binary_and_invalid_utf8_rejected() {
    let env = TestEnv::new("read_chunk_invalid");
    let cfg = env.config_dir();
    let dir = env.new_dir("project");

    // 1. Raw binary bytes
    let bin_file = dir.join("binary.bin");
    std::fs::write(&bin_file, &[0xFF, 0xFE, 0xBA, 0xBE]).unwrap();
    let canon_bin = dunce::canonicalize(&bin_file).unwrap();
    let err_bin =
        read_local_file_chunk_impl(&cfg, canon_bin.to_str().unwrap(), 0, None).unwrap_err();
    assert_eq!(err_bin, "File contains binary or unsupported text encoding");

    // 2. Embedded null byte
    let null_file = dir.join("null.txt");
    std::fs::write(&null_file, b"Hello\0World").unwrap();
    let canon_null = dunce::canonicalize(&null_file).unwrap();
    let err_null =
        read_local_file_chunk_impl(&cfg, canon_null.to_str().unwrap(), 0, None).unwrap_err();
    assert_eq!(
        err_null,
        "File contains binary or unsupported text encoding"
    );

    // 3. Truncated UTF-8 at EOF
    let trunc_file = dir.join("trunc.txt");
    std::fs::write(&trunc_file, &[0x41, 0x42, 0xE9]).unwrap(); // 'A', 'B', incomplete 3-byte lead
    let canon_trunc = dunce::canonicalize(&trunc_file).unwrap();
    let err_trunc =
        read_local_file_chunk_impl(&cfg, canon_trunc.to_str().unwrap(), 0, None).unwrap_err();
    assert_eq!(
        err_trunc,
        "File contains binary or unsupported text encoding"
    );
}

#[test]
fn test_read_local_file_chunk_sensitive_and_secret_rejected() {
    let env = TestEnv::new("read_chunk_sensitive");
    let cfg = env.config_dir();
    let dir = env.new_dir("project");

    let secret_names = [
        ".env",
        ".env.local",
        "credentials.json",
        "id_rsa",
        "id_ed25519",
        "private.key",
        "cert.pem",
        ".npmrc",
        ".pypirc",
        ".netrc",
        "secrets.json",
    ];

    for name in &secret_names {
        let path = dir.join(name);
        std::fs::write(&path, b"SECRET_VALUE=123").unwrap();
        let canon = dunce::canonicalize(&path).unwrap();
        let err = read_local_file_chunk_impl(&cfg, canon.to_str().unwrap(), 0, None).unwrap_err();
        assert_eq!(err, "Access to restricted folder or file is blocked");
    }

    // Inside secret directories
    let secret_dirs = [".ssh", ".aws", ".azure", ".kube", ".gnupg", ".docker"];
    for sdir in &secret_dirs {
        let folder = dir.join(sdir);
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("config");
        std::fs::write(&file, b"secret config").unwrap();
        let canon = dunce::canonicalize(&file).unwrap();
        let err = read_local_file_chunk_impl(&cfg, canon.to_str().unwrap(), 0, None).unwrap_err();
        assert_eq!(err, "Access to restricted folder or file is blocked");
    }
}

#[test]
fn test_read_local_file_chunk_system_directory_rejected() {
    let env = TestEnv::new("read_chunk_system");
    let cfg = env.config_dir();

    #[cfg(windows)]
    {
        if let Some(windir) = std::env::var_os("windir") {
            let win_hosts = PathBuf::from(windir)
                .join("System32")
                .join("drivers")
                .join("etc")
                .join("hosts");
            if win_hosts.exists() {
                let canon = dunce::canonicalize(&win_hosts).unwrap();
                let err =
                    read_local_file_chunk_impl(&cfg, canon.to_str().unwrap(), 0, None).unwrap_err();
                assert_eq!(err, "Access to system directory is blocked");
            }
        }
    }

    // Yuki configuration directory
    let cfg_file = cfg.join("settings.json");
    std::fs::write(&cfg_file, b"{}").unwrap();
    let canon_cfg = dunce::canonicalize(&cfg_file).unwrap();
    let err_cfg =
        read_local_file_chunk_impl(&cfg, canon_cfg.to_str().unwrap(), 0, None).unwrap_err();
    assert_eq!(err_cfg, "Access to application data or settings is blocked");

    // Yuki data root
    let data_dir = env.new_dir("data_storage");
    choose_data_root_path(&cfg, &data_dir).unwrap();
    let yuki_data_file = data_dir.join("Yuki").join("history.json");
    std::fs::write(&yuki_data_file, b"{}").unwrap();
    let canon_data = dunce::canonicalize(&yuki_data_file).unwrap();
    let err_data =
        read_local_file_chunk_impl(&cfg, canon_data.to_str().unwrap(), 0, None).unwrap_err();
    assert_eq!(
        err_data,
        "Access to application data or settings is blocked"
    );
}

#[test]
fn test_read_local_file_chunk_symlink_safety() {
    let env = TestEnv::new("read_chunk_symlink");
    let cfg = env.config_dir();
    let project = env.new_dir("project");
    let outside = env.new_dir("outside");

    let secret_file = outside.join(".env");
    std::fs::write(&secret_file, b"DATABASE_URL=secret").unwrap();

    let link_path = project.join("symlink_to_secret.txt");

    #[cfg(windows)]
    let link_created = std::os::windows::fs::symlink_file(&secret_file, &link_path).is_ok();
    #[cfg(unix)]
    let link_created = std::os::unix::fs::symlink(&secret_file, &link_path).is_ok();

    if link_created {
        // Attempt to read file via symlink pointing to secret file must fail
        let err =
            read_local_file_chunk_impl(&cfg, link_path.to_str().unwrap(), 0, None).unwrap_err();
        assert_eq!(err, "Access to restricted folder or file is blocked");
    }
}

#[test]
fn test_provider_sharing_toggle_and_revocation_clears_grant() {
    let env = TestEnv::new("provider_sharing_grants");
    let cfg = env.config_dir();
    let project = env.new_dir("ProjectAlpha");
    let file = project.join("app.rs");
    std::fs::write(&file, b"fn main() {}").unwrap();

    // 1. Add project root -> default-off provider sharing
    let settings = add_project_root_path(&cfg, &project).unwrap();
    let canon_root = dunce::canonicalize(&project)
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert_eq!(settings.project_roots, vec![canon_root.clone()]);
    assert!(
        settings.provider_sharing_grants.is_empty(),
        "Provider sharing must default to off"
    );

    // authorize_provider_file must fail because sharing is not enabled
    let err_unauthorized = authorize_provider_file(&cfg, &file).unwrap_err();
    assert_eq!(
        err_unauthorized,
        "Provider sharing is not enabled for this project folder"
    );

    // 2. Toggle provider sharing ON
    let settings_on = toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();
    assert_eq!(
        settings_on.provider_sharing_grants,
        vec![canon_root.clone()]
    );

    // Now authorize_provider_file must succeed!
    let authorized = authorize_provider_file(&cfg, &file).unwrap();
    let canon_file = dunce::canonicalize(&file).unwrap();
    assert_eq!(authorized, canon_file);

    // 3. Toggle provider sharing OFF with flip (None)
    let settings_off = toggle_project_provider_sharing_impl(&cfg, &canon_root, None).unwrap();
    assert!(settings_off.provider_sharing_grants.is_empty());
    assert!(authorize_provider_file(&cfg, &file).is_err());

    // 4. Toggle back ON, then revoke root: grant must be cleared!
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();
    let settings_revoked = remove_project_root_impl(&cfg, &canon_root).unwrap();
    assert!(settings_revoked.project_roots.is_empty());
    assert!(
        settings_revoked.provider_sharing_grants.is_empty(),
        "Grant must be cleared upon revocation"
    );

    // Authorizing now fails with outside approved project folders
    let err_revoked = authorize_provider_file(&cfg, &file).unwrap_err();
    assert_eq!(err_revoked, "File is outside approved project folders");

    // 5. Trying to toggle unapproved root fails
    let unapproved = env.new_dir("UnapprovedDir");
    let err_toggle_unapproved =
        toggle_project_provider_sharing_impl(&cfg, unapproved.to_str().unwrap(), Some(true))
            .unwrap_err();
    assert_eq!(
        err_toggle_unapproved,
        "Cannot configure provider sharing for unapproved project folder"
    );
}

#[test]
fn test_provider_sharing_legacy_settings_compatibility() {
    let env = TestEnv::new("legacy_settings");
    let cfg = env.config_dir();
    let project = env.new_dir("LegacyProj");
    let canon_proj = dunce::canonicalize(&project)
        .unwrap()
        .to_string_lossy()
        .to_string();

    // Write a legacy settings.json without providerSharingGrants field
    let legacy_json = format!(
        r#"{{"dataRoot": null, "projectRoots": ["{}"]}}"#,
        canon_proj.replace('\\', "\\\\")
    );
    std::fs::write(cfg.join("settings.json"), legacy_json.as_bytes()).unwrap();

    let loaded = load_settings(&cfg).unwrap();
    assert_eq!(loaded.project_roots, vec![canon_proj]);
    assert!(
        loaded.provider_sharing_grants.is_empty(),
        "Legacy settings must default provider_sharing_grants to empty"
    );
}

#[test]
fn test_read_local_file_chunk_rejected_args() {
    let env = TestEnv::new("read_chunk_rejected_args");
    let cfg = env.config_dir();
    let dir = env.new_dir("project");
    let file = dir.join("test.txt");
    std::fs::write(&file, b"content").unwrap();
    let canon = dunce::canonicalize(&file).unwrap();
    let path_str = canon.to_str().unwrap();

    // 1. Empty path
    assert_eq!(
        read_local_file_chunk_impl(&cfg, "", 0, None).unwrap_err(),
        "Path cannot be empty"
    );

    // 2. Relative path
    assert_eq!(
        read_local_file_chunk_impl(&cfg, "test.txt", 0, None).unwrap_err(),
        "Path must be absolute"
    );

    // 3. Chunk size 0
    assert_eq!(
        read_local_file_chunk_impl(&cfg, path_str, 0, Some(0)).unwrap_err(),
        "Invalid chunk size: must be greater than zero"
    );

    // 4. Offset exceeds file size
    assert_eq!(
        read_local_file_chunk_impl(&cfg, path_str, 100, None).unwrap_err(),
        "Offset exceeds file size"
    );

    // 5. Target is a directory
    let dir_canon = dunce::canonicalize(&dir).unwrap();
    assert_eq!(
        read_local_file_chunk_impl(&cfg, dir_canon.to_str().unwrap(), 0, None).unwrap_err(),
        "Target is not a regular file"
    );

    // 6. DOS device names and UNC paths
    #[cfg(windows)]
    {
        assert_eq!(
            read_local_file_chunk_impl(&cfg, "C:\\NUL", 0, None).unwrap_err(),
            "Device and UNC paths are not permitted"
        );
        assert_eq!(
            read_local_file_chunk_impl(&cfg, "C:\\CON.txt", 0, None).unwrap_err(),
            "Device and UNC paths are not permitted"
        );
        assert_eq!(
            read_local_file_chunk_impl(&cfg, "\\\\server\\share\\file.txt", 0, None).unwrap_err(),
            "Device and UNC paths are not permitted"
        );
    }
}

#[test]
fn test_authorize_provider_file_grants_and_containment() {
    let env = TestEnv::new("auth_provider_containment");
    let cfg = env.config_dir();
    let project = env.new_dir("ProjectBeta");
    let sub = project.join("subfolder");
    std::fs::create_dir_all(&sub).unwrap();
    let file = sub.join("module.rs");
    std::fs::write(&file, b"pub fn code() {}").unwrap();

    add_project_root_path(&cfg, &project).unwrap();
    let canon_proj = dunce::canonicalize(&project)
        .unwrap()
        .to_string_lossy()
        .to_string();

    // Sharing grant enabled
    toggle_project_provider_sharing_impl(&cfg, &canon_proj, Some(true)).unwrap();

    // Ordinary file within project root is authorized
    let auth = authorize_provider_file(&cfg, &file).unwrap();
    assert_eq!(auth, dunce::canonicalize(&file).unwrap());

    // File outside project root is not authorized
    let outside = env.new_dir("outside");
    let outside_file = outside.join("other.rs");
    std::fs::write(&outside_file, b"fn other() {}").unwrap();
    let err_out = authorize_provider_file(&cfg, &outside_file).unwrap_err();
    assert_eq!(err_out, "File is outside approved project folders");

    // But ordinary read succeeds for outside file! (broad read doesn't require project root)
    let outside_canon = dunce::canonicalize(&outside_file).unwrap();
    let read_res = read_local_file_chunk_impl(&cfg, outside_canon.to_str().unwrap(), 0, None);
    assert!(
        read_res.is_ok(),
        "Broad ordinary reads do not require project-root approval"
    );
}

#[test]
fn test_get_shareable_project_roots_impl() {
    let env = TestEnv::new("shareable_roots");
    let cfg = env.config_dir();
    let dir1 = env.new_dir("proj1");
    let dir2 = env.new_dir("proj2");

    // Initially no project roots
    let roots = get_shareable_project_roots_impl(&cfg).unwrap();
    assert!(roots.is_empty());

    // Add proj1 and proj2, neither has sharing grant
    add_project_root_path(&cfg, &dir1).unwrap();
    add_project_root_path(&cfg, &dir2).unwrap();

    let roots = get_shareable_project_roots_impl(&cfg).unwrap();
    assert!(
        roots.is_empty(),
        "Roots without grants must not be shareable"
    );

    // Enable sharing on proj1
    let canon1 = dunce::canonicalize(&dir1)
        .unwrap()
        .to_string_lossy()
        .to_string();
    toggle_project_provider_sharing_impl(&cfg, &canon1, Some(true)).unwrap();

    let roots = get_shareable_project_roots_impl(&cfg).unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0], canon1);

    // Enable sharing on proj2
    let canon2 = dunce::canonicalize(&dir2)
        .unwrap()
        .to_string_lossy()
        .to_string();
    toggle_project_provider_sharing_impl(&cfg, &canon2, Some(true)).unwrap();

    let roots = get_shareable_project_roots_impl(&cfg).unwrap();
    assert_eq!(roots.len(), 2);

    // Revoke proj1
    toggle_project_provider_sharing_impl(&cfg, &canon1, Some(false)).unwrap();
    let roots = get_shareable_project_roots_impl(&cfg).unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0], canon2);
}

#[test]
fn test_list_provider_project_directory_impl() {
    let env = TestEnv::new("provider_list_dir");
    let cfg = env.config_dir();
    let proj = env.new_dir("proj");
    let unapproved = env.new_dir("unapproved");

    std::fs::write(proj.join("main.rs"), b"fn main() {}").unwrap();
    std::fs::write(proj.join(".env"), b"SECRET=123").unwrap();
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::write(proj.join("src").join("lib.rs"), b"pub fn f() {}").unwrap();

    let canon_proj = dunce::canonicalize(&proj)
        .unwrap()
        .to_string_lossy()
        .to_string();

    // 1. Unapproved root rejected
    let err_unapproved =
        list_provider_project_directory_impl(&cfg, unapproved.to_str().unwrap(), "").unwrap_err();
    assert_eq!(
        err_unapproved,
        "Root directory is not an approved project folder"
    );

    // 2. Approved root but sharing NOT enabled
    add_project_root_path(&cfg, &proj).unwrap();
    let err_ungranted = list_provider_project_directory_impl(&cfg, &canon_proj, "").unwrap_err();
    assert_eq!(
        err_ungranted,
        "Provider sharing is not enabled for this project folder"
    );

    // 3. Enable sharing -> directory listing succeeds
    toggle_project_provider_sharing_impl(&cfg, &canon_proj, Some(true)).unwrap();
    let entries = list_provider_project_directory_impl(&cfg, &canon_proj, "").unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"main.rs"));
    assert!(names.contains(&"src"));
    assert!(
        !names.contains(&".env"),
        "Secret files must be excluded from provider listing"
    );

    // Subdirectory listing
    let sub_entries = list_provider_project_directory_impl(&cfg, &canon_proj, "src").unwrap();
    assert_eq!(sub_entries.len(), 1);
    assert_eq!(sub_entries[0].name, "lib.rs");

    // Traversal rejection
    let err_traversal = list_provider_project_directory_impl(&cfg, &canon_proj, "../").unwrap_err();
    assert_eq!(err_traversal, "Path traversal (..) is not allowed");

    // 4. Revocation immediately disables listing
    toggle_project_provider_sharing_impl(&cfg, &canon_proj, Some(false)).unwrap();
    let err_revoked = list_provider_project_directory_impl(&cfg, &canon_proj, "").unwrap_err();
    assert_eq!(
        err_revoked,
        "Provider sharing is not enabled for this project folder"
    );
}

// ---------------------------------------------------------------------------
// Game Project Discovery Tests
// ---------------------------------------------------------------------------

#[test]
fn test_identify_game_project_unity() {
    let env = TestEnv::new("id_unity");
    let proj = env.new_dir("UnityTest");
    let canon_proj = dunce::canonicalize(&proj).unwrap();
    let root_str = canon_proj.to_string_lossy().to_string();

    // Initially empty - not a project
    assert!(identify_game_project(&proj, &canon_proj, &root_str).is_none());

    // Only ProjectSettings/ProjectVersion.txt without Assets/ -> not a project
    let settings_dir = proj.join("ProjectSettings");
    std::fs::create_dir_all(&settings_dir).unwrap();
    std::fs::write(
        settings_dir.join("ProjectVersion.txt"),
        b"m_EditorVersion: 2022.3.15f1\nm_EditorVersionWithRevision: 2022.3.15f1 (abcdef)",
    )
    .unwrap();
    assert!(identify_game_project(&proj, &canon_proj, &root_str).is_none());

    // Add Assets/ directory -> valid Unity project
    std::fs::create_dir_all(proj.join("Assets")).unwrap();
    let discovered = identify_game_project(&proj, &canon_proj, &root_str)
        .expect("Must identify as Unity project");
    assert_eq!(discovered.engine, "Unity");
    assert_eq!(discovered.version.as_deref(), Some("2022.3.15f1"));
    assert_eq!(discovered.name, "UnityTest");
    assert!(
        discovered
            .evidence
            .contains(&"ProjectSettings/ProjectVersion.txt".to_string())
    );
    assert!(discovered.evidence.contains(&"Assets".to_string()));
}

#[test]
fn test_identify_game_project_godot() {
    let env = TestEnv::new("id_godot");
    let proj = env.new_dir("GodotTest");
    let canon_proj = dunce::canonicalize(&proj).unwrap();
    let root_str = canon_proj.to_string_lossy().to_string();

    // Initially empty
    assert!(identify_game_project(&proj, &canon_proj, &root_str).is_none());

    // Add project.godot
    let godot_cfg = r#"; Engine configuration file.
config_version=5

[application]
config/name="Super Space Explorer"
run/main_scene="res://main.tscn"
"#;
    std::fs::write(proj.join("project.godot"), godot_cfg).unwrap();

    let discovered = identify_game_project(&proj, &canon_proj, &root_str)
        .expect("Must identify as Godot project");
    assert_eq!(discovered.engine, "Godot");
    assert_eq!(discovered.version.as_deref(), Some("4.x"));
    assert_eq!(discovered.name, "Super Space Explorer");
    assert!(discovered.evidence.contains(&"project.godot".to_string()));
}

#[test]
fn test_identify_game_project_unreal() {
    let env = TestEnv::new("id_unreal");
    let proj = env.new_dir("UnrealTest");
    let canon_proj = dunce::canonicalize(&proj).unwrap();
    let root_str = canon_proj.to_string_lossy().to_string();

    // Initially empty
    assert!(identify_game_project(&proj, &canon_proj, &root_str).is_none());

    // Add .uproject
    let uproject_content = r#"{
    "FileVersion": 3,
    "EngineAssociation": "5.4",
    "Category": "",
    "Description": ""
}"#;
    std::fs::write(proj.join("FantasyQuest.uproject"), uproject_content).unwrap();

    let discovered = identify_game_project(&proj, &canon_proj, &root_str)
        .expect("Must identify as Unreal project");
    assert_eq!(discovered.engine, "Unreal");
    assert_eq!(discovered.version.as_deref(), Some("5.4"));
    assert_eq!(discovered.name, "FantasyQuest");
    assert!(
        discovered
            .evidence
            .contains(&"FantasyQuest.uproject".to_string())
    );
}

#[test]
fn test_discover_game_projects_multi_engine_and_exclusions() {
    let env = TestEnv::new("discover_multi");
    let cfg = env.config_dir();
    let root = env.new_dir("RootGames");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    // 1. Unity project with cache folders
    let unity_dir = root.join("MyUnityGame");
    std::fs::create_dir_all(unity_dir.join("ProjectSettings")).unwrap();
    std::fs::write(
        unity_dir.join("ProjectSettings").join("ProjectVersion.txt"),
        b"m_EditorVersion: 2021.3.8f1\n",
    )
    .unwrap();
    std::fs::create_dir_all(unity_dir.join("Assets")).unwrap();
    // Cache folder inside Unity - should NOT be traversed into
    let unity_cache = unity_dir.join("Library").join("PackageCache");
    std::fs::create_dir_all(&unity_cache).unwrap();
    std::fs::write(unity_cache.join("dummy.txt"), b"cache").unwrap();

    // 2. Godot project with cache folder
    let godot_dir = root.join("MyGodotGame");
    std::fs::create_dir_all(&godot_dir).unwrap();
    std::fs::write(
        godot_dir.join("project.godot"),
        b"config_version=4\n[application]\nconfig/name=\"RetroRacer\"\n",
    )
    .unwrap();
    let godot_cache = godot_dir.join(".godot");
    std::fs::create_dir_all(&godot_cache).unwrap();

    // 3. Unreal project with cache folder
    let unreal_dir = root.join("MyUnrealGame");
    std::fs::create_dir_all(&unreal_dir).unwrap();
    std::fs::write(
        unreal_dir.join("Arena.uproject"),
        b"{\"EngineAssociation\": \"5.3\"}",
    )
    .unwrap();
    let unreal_cache = unreal_dir.join("Intermediate");
    std::fs::create_dir_all(&unreal_cache).unwrap();

    // 4. Secret directory and files - should be excluded
    let git_dir = root.join(".git");
    std::fs::create_dir_all(&git_dir).unwrap();
    // Put a fake project file inside .git to ensure it is never discovered
    std::fs::write(git_dir.join("project.godot"), b"config_version=5\n").unwrap();
    std::fs::write(root.join(".env"), b"SECRET=abc").unwrap();

    // 5. Non-game directory
    let docs_dir = root.join("docs");
    std::fs::create_dir_all(&docs_dir).unwrap();
    std::fs::write(docs_dir.join("manual.md"), b"# Docs").unwrap();

    // Run discovery
    let res = discover_game_projects_impl(&cfg, "{}").expect("Discovery should succeed");

    assert_eq!(
        res.projects.len(),
        3,
        "Must discover exactly the 3 game projects"
    );
    let names: Vec<&str> = res.projects.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"MyUnityGame"));
    assert!(names.contains(&"RetroRacer"));
    assert!(names.contains(&"Arena"));

    assert_eq!(res.coverage, "complete");
    assert_eq!(res.limits_reached.len(), 0);
    assert!(!res.truncated);
    assert!(res.directories_scanned > 0);
    assert!(res.scanned_roots.contains(&canon_root));
}

#[test]
fn test_discover_game_projects_unapproved_root_and_revocation() {
    let env = TestEnv::new("discover_unapproved_revocation");
    let cfg = env.config_dir();

    let root_a = env.new_dir("RootA");
    let root_b = env.new_dir("RootB");
    let unapproved = env.new_dir("UnapprovedRoot");

    let canon_a = dunce::canonicalize(&root_a)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let canon_b = dunce::canonicalize(&root_b)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let canon_unapproved = dunce::canonicalize(&unapproved)
        .unwrap()
        .to_string_lossy()
        .to_string();

    // Setup projects in RootA and RootB
    let proj_a = root_a.join("GameA");
    std::fs::create_dir_all(&proj_a).unwrap();
    std::fs::write(proj_a.join("project.godot"), b"config_version=5\n").unwrap();

    let proj_b = root_b.join("GameB");
    std::fs::create_dir_all(&proj_b).unwrap();
    std::fs::write(proj_b.join("GameB.uproject"), b"{}").unwrap();

    // 1. Unapproved root rejected
    let unapproved_args = serde_json::json!({
        "project_root": canon_unapproved,
    })
    .to_string();
    let err_unapproved = discover_game_projects_impl(&cfg, &unapproved_args).unwrap_err();
    assert!(err_unapproved.contains(
        "Root directory is not an approved project folder or provider sharing is not enabled"
    ));

    // 2. Approved but unshared root rejected
    add_project_root_path(&cfg, &root_a).unwrap();
    let unshared_args = serde_json::json!({
        "project_root": canon_a,
    })
    .to_string();
    let err_unshared = discover_game_projects_impl(&cfg, &unshared_args).unwrap_err();
    assert!(err_unshared.contains(
        "Root directory is not an approved project folder or provider sharing is not enabled"
    ));

    // 3. Enable sharing on both RootA and RootB
    add_project_root_path(&cfg, &root_b).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_a, Some(true)).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_b, Some(true)).unwrap();

    let discovery_res = discover_game_projects_impl(&cfg, "{}").unwrap();
    assert_eq!(discovery_res.projects.len(), 2);

    let mut serialized_output = serde_json::to_string(&discovery_res).unwrap();

    // 4. Revocation of RootA: reauthorize_discovery_result prunes RootA projects
    toggle_project_provider_sharing_impl(&cfg, &canon_a, Some(false)).unwrap();
    reauthorize_discovery_result(&cfg, &mut serialized_output);
    let reauth_val: serde_json::Value = serde_json::from_str(&serialized_output).unwrap();
    let reauth_projects = reauth_val["projects"].as_array().unwrap();
    assert_eq!(reauth_projects.len(), 1);
    assert_eq!(reauth_projects[0]["name"], "GameB");
    let limits = reauth_val["limitsReached"].as_array().unwrap();
    assert!(limits.iter().any(|v| v == "revoked_grant"));

    // 5. Revocation of RootB as well: all roots revoked -> error
    toggle_project_provider_sharing_impl(&cfg, &canon_b, Some(false)).unwrap();
    reauthorize_discovery_result(&cfg, &mut serialized_output);
    assert!(
        serialized_output.contains("Provider sharing was revoked for all scanned project roots")
    );
}

#[test]
fn test_discover_game_projects_depth_and_limits() {
    let env = TestEnv::new("discover_depth_limits");
    let cfg = env.config_dir();
    let root = env.new_dir("NestedRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    // Create deep structure: depth 5
    let deep_dir = root.join("l1").join("l2").join("l3").join("l4").join("l5");
    std::fs::create_dir_all(&deep_dir).unwrap();
    std::fs::write(deep_dir.join("project.godot"), b"config_version=5\n").unwrap();

    // Discover with max_depth = 2
    let args = serde_json::json!({
        "max_depth": 2
    })
    .to_string();
    let res = discover_game_projects_impl(&cfg, &args).unwrap();
    assert_eq!(res.projects.len(), 0, "Should not reach depth 5 project");
    assert_eq!(res.coverage, "partial");
    assert!(res.truncated);
    assert!(res.limits_reached.contains(&"max_depth".to_string()));
}

// ---------------------------------------------------------------------------
// Search Tools Native Unit & Integration Tests
// ---------------------------------------------------------------------------

#[test]
fn test_glob_match_patterns_and_metacharacters() {
    // Basic exact match
    assert!(glob_match("file.txt", "file.txt", true));
    assert!(!glob_match("file.txt", "file.doc", true));

    // Case sensitivity
    assert!(glob_match("File.TXT", "file.txt", false));
    assert!(!glob_match("File.TXT", "file.txt", true));

    // Wildcard *
    assert!(glob_match("*.cs", "Player.cs", false));
    assert!(glob_match("*.cs", "Player.CS", false));
    assert!(!glob_match("*.cs", "Player.cs.bak", false));
    assert!(glob_match("Player*.cs", "PlayerController.cs", false));
    assert!(glob_match(
        "Player*Controller*.cs",
        "PlayerMovementController2D.cs",
        false
    ));
    assert!(glob_match("*test*", "my_test_runner.rs", false));
    assert!(glob_match("*", "anything.txt", false));

    // Wildcard ?
    assert!(glob_match("file?.txt", "file1.txt", false));
    assert!(glob_match("file?.txt", "fileA.txt", false));
    assert!(!glob_match("file?.txt", "file12.txt", false));
    assert!(!glob_match("file?.txt", "file.txt", false));

    // Unicode in pattern and text
    assert!(glob_match("*雪*.txt", "冬の雪景色.txt", false));
    assert!(glob_match("*🦀*", "rust_🦀_test", false));
    assert!(!glob_match("*雪*.txt", "雨の日.txt", false));
}

#[test]
fn test_matches_file_pattern_literal_vs_glob() {
    // Literal mode treats metacharacters as exact literal characters
    assert!(matches_file_pattern(
        "test*.cs",
        "test*.cs",
        "test*.cs",
        FileMatchType::Literal,
        true
    ));
    assert!(!matches_file_pattern(
        "testing.cs",
        "testing.cs",
        "test*.cs",
        FileMatchType::Literal,
        true
    ));

    assert!(matches_file_pattern(
        "[0].txt",
        "[0].txt",
        "[0].txt",
        FileMatchType::Literal,
        true
    ));
    assert!(!matches_file_pattern(
        "0.txt",
        "0.txt",
        "[0].txt",
        FileMatchType::Literal,
        true
    ));

    assert!(matches_file_pattern(
        "Player.cs",
        "Player.cs",
        "Player",
        FileMatchType::Literal,
        false
    ));
    assert!(matches_file_pattern(
        "player.cs",
        "player.cs",
        "Player",
        FileMatchType::Literal,
        false
    ));
    assert!(!matches_file_pattern(
        "player.cs",
        "player.cs",
        "Player",
        FileMatchType::Literal,
        true
    ));

    // Path matching with slashes
    assert!(matches_file_pattern(
        "Player.cs",
        "Scripts/Player.cs",
        "Scripts/Player",
        FileMatchType::Literal,
        false
    ));
    assert!(matches_file_pattern(
        "Player.cs",
        "Scripts\\Player.cs",
        "Scripts/Player",
        FileMatchType::Literal,
        false
    ));

    // Glob mode
    assert!(matches_file_pattern(
        "Player.cs",
        "Scripts/Player.cs",
        "*.cs",
        FileMatchType::Glob,
        false
    ));
    assert!(matches_file_pattern(
        "Player.cs",
        "Scripts/Player.cs",
        "Scripts/*.cs",
        FileMatchType::Glob,
        false
    ));
    assert!(!matches_file_pattern(
        "Player.cs",
        "Assets/Player.cs",
        "Scripts/*.cs",
        FileMatchType::Glob,
        false
    ));
}

#[test]
fn test_search_files_permissions_and_exclusions() {
    let env = TestEnv::new("search_files_perms");
    let cfg = env.config_dir();
    let root = env.new_dir("SearchRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    // 1. Unapproved root rejected
    let unapproved_args = serde_json::json!({
        "pattern": "*.cs",
        "project_root": canon_root
    })
    .to_string();
    let err_unapproved = search_project_files_impl(&cfg, &unapproved_args).unwrap_err();
    assert!(err_unapproved.contains("Access denied"));

    // 2. Approved but ungranted root rejected
    add_project_root_path(&cfg, &root).unwrap();
    let err_ungranted = search_project_files_impl(&cfg, &unapproved_args).unwrap_err();
    assert!(err_ungranted.contains("Access denied"));

    // 3. Enable sharing
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    // Populate directory with normal, secret, cache, and link entries
    let normal_file = root.join("PlayerController.cs");
    std::fs::write(&normal_file, b"// normal").unwrap();

    let sub_dir = root.join("Assets");
    std::fs::create_dir_all(&sub_dir).unwrap();
    let sub_file = sub_dir.join("Enemy.cs");
    std::fs::write(&sub_file, b"// enemy").unwrap();

    // Secret files that must NEVER be returned
    let env_file = root.join(".env");
    std::fs::write(&env_file, b"SECRET=123").unwrap();
    let env_local = root.join(".env.local");
    std::fs::write(&env_local, b"SECRET=local").unwrap();
    let creds_file = root.join("credentials.json");
    std::fs::write(&creds_file, b"{}").unwrap();
    let key_file = root.join("server.pem");
    std::fs::write(&key_file, b"KEY").unwrap();
    let secrets_file = root.join("secrets.yaml");
    std::fs::write(&secrets_file, b"secrets: true").unwrap();

    // Secret directory that must NEVER be traversed
    let ssh_dir = root.join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();
    let ssh_key = ssh_dir.join("id_rsa.cs"); // Even if named .cs!
    std::fs::write(&ssh_key, b"SSH").unwrap();

    // Cache directory that must NEVER be traversed
    let lib_dir = root.join("Library");
    std::fs::create_dir_all(&lib_dir).unwrap();
    let cache_file = lib_dir.join("Cached.cs");
    std::fs::write(&cache_file, b"cache").unwrap();

    // Search for *.cs with glob
    let args = serde_json::json!({
        "pattern": "*.cs",
        "match_type": "glob",
        "project_root": canon_root
    })
    .to_string();

    let res = search_project_files_impl(&cfg, &args).unwrap();
    assert_eq!(res.coverage, "complete");
    assert!(!res.truncated);
    assert_eq!(res.matches.len(), 2);

    let match_names: Vec<&str> = res.matches.iter().map(|m| m.name.as_str()).collect();
    assert!(match_names.contains(&"PlayerController.cs"));
    assert!(match_names.contains(&"Enemy.cs"));
    assert!(
        !match_names.contains(&"id_rsa.cs"),
        "Secret directory files must be excluded"
    );
    assert!(
        !match_names.contains(&"Cached.cs"),
        "Cache directory files must be excluded"
    );

    // Search specifically for secrets: should find 0 matches
    let secret_args = serde_json::json!({
        "pattern": "*env*",
        "match_type": "glob",
        "project_root": canon_root
    })
    .to_string();
    let secret_res = search_project_files_impl(&cfg, &secret_args).unwrap();
    assert_eq!(
        secret_res.matches.len(),
        0,
        "Secret files must be strictly excluded"
    );
}

#[test]
fn test_search_files_literal_metacharacters_and_modes() {
    let env = TestEnv::new("search_files_modes");
    let cfg = env.config_dir();
    let root = env.new_dir("ModesRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    // Create files with literal metacharacters and names
    std::fs::write(root.join("test_special.cs"), b"special").unwrap();
    std::fs::write(root.join("test_normal.cs"), b"normal").unwrap();
    std::fs::write(root.join("[0]_item.json"), b"brackets").unwrap();
    std::fs::write(root.join("CaseSensitive.txt"), b"case").unwrap();

    // 1. Literal search for "test_*.cs": should match 0 files because '*' is treated literally
    let lit_args = serde_json::json!({
        "pattern": "test_*.cs",
        "match_type": "literal",
        "project_root": canon_root
    })
    .to_string();
    let lit_res = search_project_files_impl(&cfg, &lit_args).unwrap();
    assert_eq!(lit_res.matches.len(), 0);

    // 2. Glob search for "test_*.cs": matches BOTH "test_special.cs" and "test_normal.cs"
    let glob_args = serde_json::json!({
        "pattern": "test_*.cs",
        "match_type": "glob",
        "project_root": canon_root
    })
    .to_string();
    let glob_res = search_project_files_impl(&cfg, &glob_args).unwrap();
    assert_eq!(glob_res.matches.len(), 2);

    // 3. Literal search for "[0]" (bracket characters that regex would treat as char class)
    let bracket_args = serde_json::json!({
        "pattern": "[0]",
        "match_type": "literal",
        "project_root": canon_root
    })
    .to_string();
    let bracket_res = search_project_files_impl(&cfg, &bracket_args).unwrap();
    assert_eq!(bracket_res.matches.len(), 1);
    assert_eq!(bracket_res.matches[0].name, "[0]_item.json");

    // 4. Case-sensitivity toggle
    let case_off_args = serde_json::json!({
        "pattern": "casesensitive",
        "case_sensitive": false,
        "project_root": canon_root
    })
    .to_string();
    let case_off_res = search_project_files_impl(&cfg, &case_off_args).unwrap();
    assert_eq!(case_off_res.matches.len(), 1);

    let case_on_args = serde_json::json!({
        "pattern": "casesensitive",
        "case_sensitive": true,
        "project_root": canon_root
    })
    .to_string();
    let case_on_res = search_project_files_impl(&cfg, &case_on_args).unwrap();
    assert_eq!(case_on_res.matches.len(), 0);
}

#[test]
fn test_search_files_coverage_budgets_and_cursor() {
    let env = TestEnv::new("search_files_budgets");
    let cfg = env.config_dir();
    let root = env.new_dir("BudgetRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    // Create 5 files alphabetically
    for c in ['A', 'B', 'C', 'D', 'E'] {
        std::fs::write(root.join(format!("File_{c}.txt")), b"test").unwrap();
    }

    // 1. max_results = 2
    let page1_args = serde_json::json!({
        "pattern": "File_",
        "max_results": 2,
        "project_root": canon_root
    })
    .to_string();
    let page1 = search_project_files_impl(&cfg, &page1_args).unwrap();
    assert_eq!(page1.matches.len(), 2);
    assert_eq!(page1.matches[0].name, "File_A.txt");
    assert_eq!(page1.matches[1].name, "File_B.txt");
    assert_eq!(page1.coverage, "partial");
    assert!(page1.truncated);
    assert!(page1.limits_reached.contains(&"max_results".to_string()));
    assert_eq!(page1.next_cursor, Some(2));

    // 2. Cursor pagination: cursor = 2, max_results = 2
    let page2_args = serde_json::json!({
        "pattern": "File_",
        "cursor": 2,
        "max_results": 2,
        "project_root": canon_root
    })
    .to_string();
    let page2 = search_project_files_impl(&cfg, &page2_args).unwrap();
    assert_eq!(page2.matches.len(), 2);
    assert_eq!(page2.matches[0].name, "File_C.txt");
    assert_eq!(page2.matches[1].name, "File_D.txt");
    assert_eq!(page2.next_cursor, Some(4));

    // 3. Cursor pagination: cursor = 4, max_results = 2 (page 3 reaches end)
    let page3_args = serde_json::json!({
        "pattern": "File_",
        "cursor": 4,
        "max_results": 2,
        "project_root": canon_root
    })
    .to_string();
    let page3 = search_project_files_impl(&cfg, &page3_args).unwrap();
    assert_eq!(page3.matches.len(), 1);
    assert_eq!(page3.matches[0].name, "File_E.txt");
    assert_eq!(page3.coverage, "complete");
    assert!(!page3.truncated);
    assert_eq!(page3.next_cursor, None);
}

#[test]
fn test_search_files_reauthorization_revocation() {
    let env = TestEnv::new("search_files_reauth");
    let cfg = env.config_dir();
    let root1 = env.new_dir("RootOne");
    let root2 = env.new_dir("RootTwo");
    let canon_r1 = dunce::canonicalize(&root1)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let canon_r2 = dunce::canonicalize(&root2)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root1).unwrap();
    add_project_root_path(&cfg, &root2).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_r1, Some(true)).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_r2, Some(true)).unwrap();

    let file1 = root1.join("Item1.txt");
    let file2 = root2.join("Item2.txt");
    std::fs::write(&file1, b"1").unwrap();
    std::fs::write(&file2, b"2").unwrap();

    let mut res_str = execute_tool(&cfg, "search_project_files", "{\"pattern\":\"Item\"}");
    assert!(res_str.contains("Item1.txt"));
    assert!(res_str.contains("Item2.txt"));

    // Revoke root1 sharing
    toggle_project_provider_sharing_impl(&cfg, &canon_r1, Some(false)).unwrap();

    reauthorize_search_files_result(&cfg, &mut res_str);
    assert!(
        !res_str.contains("Item1.txt"),
        "Revoked root results must be pruned"
    );
    assert!(res_str.contains("Item2.txt"));
    assert!(res_str.contains("\"revoked_grant\""));
    assert!(res_str.contains("\"coverage\":\"partial\""));

    // Revoke root2 sharing (all revoked)
    toggle_project_provider_sharing_impl(&cfg, &canon_r2, Some(false)).unwrap();

    reauthorize_search_files_result(&cfg, &mut res_str);
    assert!(
        res_str.contains("error"),
        "Must replace with error payload when all roots revoked"
    );
    assert!(
        !res_str.contains("Item2.txt"),
        "No filenames or paths may leak"
    );
}

#[test]
fn test_search_text_permissions_and_exclusions() {
    let env = TestEnv::new("search_text_perms");
    let cfg = env.config_dir();
    let root = env.new_dir("TextRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    // 1. Unapproved root rejected
    let unapproved_args = serde_json::json!({
        "query": "HEALTH_KEYWORD",
        "project_root": canon_root
    })
    .to_string();
    let err_unapproved = search_project_text_impl(&cfg, &unapproved_args).unwrap_err();
    assert!(err_unapproved.contains("Access denied"));

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    // Normal file
    let normal_file = root.join("Game.cs");
    std::fs::write(&normal_file, b"public int HEALTH_KEYWORD = 100;\n").unwrap();

    // Secret file containing same keyword
    let env_file = root.join(".env");
    std::fs::write(&env_file, b"HEALTH_KEYWORD=SUPER_SECRET\n").unwrap();

    // Secret directory containing file with same keyword
    let ssh_dir = root.join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();
    std::fs::write(ssh_dir.join("config.txt"), b"HEALTH_KEYWORD in ssh\n").unwrap();

    // Cache directory containing file with same keyword
    let cache_dir = root.join("node_modules");
    std::fs::create_dir_all(&cache_dir).unwrap();
    std::fs::write(
        cache_dir.join("pkg.js"),
        b"HEALTH_KEYWORD in node_modules\n",
    )
    .unwrap();

    let args = serde_json::json!({
        "query": "HEALTH_KEYWORD",
        "project_root": canon_root
    })
    .to_string();

    let res = search_project_text_impl(&cfg, &args).unwrap();
    assert_eq!(res.matches.len(), 1);
    assert_eq!(res.matches[0].relative_path, "Game.cs");
    assert!(res.matches[0].snippet.contains("HEALTH_KEYWORD"));
    assert!(!res.matches.iter().any(|m| m.relative_path.contains(".env")));
    assert!(!res.matches.iter().any(|m| m.relative_path.contains(".ssh")));
    assert!(
        !res.matches
            .iter()
            .any(|m| m.relative_path.contains("node_modules"))
    );
}

#[test]
fn test_search_text_literal_metacharacters_and_case() {
    let env = TestEnv::new("search_text_literals");
    let cfg = env.config_dir();
    let root = env.new_dir("LiteralsRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    let code_file = root.join("code.txt");
    std::fs::write(
        &code_file,
        b"let val = arr[0];\nlet regex = foo*bar;\nlet price = $100;\nfn Greeter() { println!(\"\xd0\x9f\xd1\x80\xd0\xb8\xd0\xb2\xd0\xb5\xd1\x82\"); }\n",
    )
    .unwrap();

    // 1. Literal metacharacters "$100"
    let args1 = serde_json::json!({
        "query": "$100",
        "project_root": canon_root
    })
    .to_string();
    let res1 = search_project_text_impl(&cfg, &args1).unwrap();
    assert_eq!(res1.matches.len(), 1);
    assert_eq!(res1.matches[0].line_number, 3);
    assert!(res1.matches[0].snippet.contains("$100"));

    // 2. Literal metacharacters "arr[0];"
    let args2 = serde_json::json!({
        "query": "arr[0];",
        "project_root": canon_root
    })
    .to_string();
    let res2 = search_project_text_impl(&cfg, &args2).unwrap();
    assert_eq!(res2.matches.len(), 1);
    assert_eq!(res2.matches[0].line_number, 1);

    // 3. Literal "foo*bar" without wildcard interpretation
    let args3 = serde_json::json!({
        "query": "foo*bar",
        "project_root": canon_root
    })
    .to_string();
    let res3 = search_project_text_impl(&cfg, &args3).unwrap();
    assert_eq!(res3.matches.len(), 1);
    assert_eq!(res3.matches[0].line_number, 2);

    // 4. Unicode case-insensitive matching: Cyrillic "привет" matches "Привет"
    let args4 = serde_json::json!({
        "query": "привет", // lowercase "привет"
        "case_sensitive": false,
        "project_root": canon_root
    })
    .to_string();
    let res4 = search_project_text_impl(&cfg, &args4).unwrap();
    assert_eq!(res4.matches.len(), 1);
    assert_eq!(res4.matches[0].line_number, 4);

    // With case_sensitive: true, lowercase "привет" does NOT match uppercase "Привет"
    let args5 = serde_json::json!({
        "query": "привет",
        "case_sensitive": true,
        "project_root": canon_root
    })
    .to_string();
    let res5 = search_project_text_impl(&cfg, &args5).unwrap();
    assert_eq!(res5.matches.len(), 0);
}

#[test]
fn test_search_text_boundary_spanning_and_cross_chunk_utf8() {
    let env = TestEnv::new("search_text_boundary");
    let cfg = env.config_dir();
    let root = env.new_dir("BoundaryRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    // SEARCH_READ_CHUNK_SIZE is 32 KiB (32768 bytes).
    // Construct a file where:
    // 1. Bytes 0..32760 are filler 'A's with newlines.
    // 2. Target "BOUNDARY_SPANNING_TARGET" starts at byte 32762 and spans across 32768!
    let mut content = Vec::new();
    for i in 0..1000 {
        content
            .extend_from_slice(format!("Line_{i:04}: aaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n").as_bytes());
    }
    // Truncate or pad to exactly byte 32760
    content.truncate(32760);
    while content.len() < 32760 {
        content.push(b'x');
    }
    content.push(b'\n'); // byte 32760 is newline

    // Place target: "CROSS_CHUNK_KEYWORD" (19 bytes) starting at byte 32761
    // Byte 32761..32768 (7 bytes in chunk 1: "CROSS_C")
    // Byte 32768..32780 (12 bytes in chunk 2: "HUNK_KEYWORD")
    content.extend_from_slice(b"CROSS_CHUNK_KEYWORD\n");

    // Add multi-byte UTF-8 character spanning boundary in another section:
    // Append 32740 bytes of filler, then place 3-byte Japanese character '雪' (0xE9 0x9B 0xAA)
    // right across chunk boundary!
    let target_offset = 65535; // boundary between chunk 2 and chunk 3
    while content.len() < target_offset {
        content.push(b'y');
    }
    content.extend_from_slice("雪の結晶\n".as_bytes());

    let boundary_file = root.join("boundary.txt");
    std::fs::write(&boundary_file, &content).unwrap();

    // 1. Search for boundary-spanning keyword
    let args1 = serde_json::json!({
        "query": "CROSS_CHUNK_KEYWORD",
        "project_root": canon_root
    })
    .to_string();
    let res1 = search_project_text_impl(&cfg, &args1).unwrap();
    assert_eq!(
        res1.matches.len(),
        1,
        "Must find match that spans chunk boundaries"
    );
    assert_eq!(res1.matches[0].byte_offset, 32761);
    assert!(res1.matches[0].snippet.contains("CROSS_CHUNK_KEYWORD"));

    // 2. Search for keyword after split multi-byte UTF-8 character
    let args2 = serde_json::json!({
        "query": "雪の結晶",
        "project_root": canon_root
    })
    .to_string();
    let res2 = search_project_text_impl(&cfg, &args2).unwrap();
    assert_eq!(
        res2.matches.len(),
        1,
        "Must cleanly decode split multi-byte UTF-8 without error"
    );
}

#[test]
fn test_search_text_large_file_and_long_lines() {
    let env = TestEnv::new("search_text_large");
    let cfg = env.config_dir();
    let root = env.new_dir("LargeRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    // 1. Large file: 128 KiB without arbitrary size rejection
    let mut large_content = Vec::new();
    for i in 0..4000 {
        if i == 2500 {
            large_content.extend_from_slice(b"Found in middle: UNIQUE_LARGE_TARGET;\n");
        } else {
            large_content.extend_from_slice(b"Normal log line entry in large file\n");
        }
    }
    let large_file = root.join("large_log.txt");
    std::fs::write(&large_file, &large_content).unwrap();

    let args_large = serde_json::json!({
        "query": "UNIQUE_LARGE_TARGET",
        "project_root": canon_root
    })
    .to_string();
    let res_large = search_project_text_impl(&cfg, &args_large).unwrap();
    assert_eq!(res_large.matches.len(), 1);
    assert_eq!(res_large.matches[0].line_number, 2501);
    assert!(res_large.matches[0].snippet.contains("UNIQUE_LARGE_TARGET"));

    // 2. Long line: 5,000 characters on a single minified line
    let mut long_line = String::new();
    long_line.push_str(&"x".repeat(2500));
    long_line.push_str("MINIFIED_MATCH_TARGET");
    long_line.push_str(&"y".repeat(2500));
    long_line.push('\n');

    let minified_file = root.join("bundle.min.js");
    std::fs::write(&minified_file, long_line.as_bytes()).unwrap();

    let args_long = serde_json::json!({
        "query": "MINIFIED_MATCH_TARGET",
        "project_root": canon_root
    })
    .to_string();
    let res_long = search_project_text_impl(&cfg, &args_long).unwrap();
    assert_eq!(res_long.matches.len(), 1);
    let snip = &res_long.matches[0].snippet;
    assert!(
        snip.len() <= 200,
        "Snippet on long line must be bounded to <= 200 chars"
    );
    assert!(snip.contains("MINIFIED_MATCH_TARGET"));
    assert!(
        snip.starts_with("... "),
        "Snippet must have prefix truncation indicator"
    );
    assert!(
        snip.ends_with(" ..."),
        "Snippet must have suffix truncation indicator"
    );
}

#[test]
fn test_search_text_binary_rejection_and_unscanned_remainder() {
    let env = TestEnv::new("search_text_binary");
    let cfg = env.config_dir();
    let root = env.new_dir("BinaryRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    // 1. Text file with target
    std::fs::write(root.join("valid1.txt"), b"HELLO_TARGET in valid file 1\n").unwrap();
    std::fs::write(root.join("valid2.txt"), b"HELLO_TARGET in valid file 2\n").unwrap();
    std::fs::write(root.join("valid3.txt"), b"HELLO_TARGET in valid file 3\n").unwrap();

    // 2. Binary file with null bytes containing same target bytes
    let mut binary_bytes = b"HELLO_TARGET in binary".to_vec();
    binary_bytes.insert(0, 0); // null byte at start
    std::fs::write(root.join("image.png"), &binary_bytes).unwrap();

    let args = serde_json::json!({
        "query": "HELLO_TARGET",
        "max_results": 2, // limit to 2 results to check unscanned remainder
        "project_root": canon_root
    })
    .to_string();

    let res = search_project_text_impl(&cfg, &args).unwrap();
    assert_eq!(res.matches.len(), 2);
    assert_eq!(res.coverage, "partial");
    assert!(res.truncated);
    assert!(res.limits_reached.contains(&"max_results".to_string()));
    assert!(
        res.binary_files_skipped >= 1,
        "Must count skipped binary file"
    );
    assert!(
        res.unscanned_files_remainder >= 1,
        "Must report unscanned files remainder"
    );

    // 3. file_pattern filter: '*.txt' should skip other file extensions
    let filter_args = serde_json::json!({
        "query": "HELLO_TARGET",
        "file_pattern": "*.txt",
        "project_root": canon_root
    })
    .to_string();
    let filter_res = search_project_text_impl(&cfg, &filter_args).unwrap();
    assert_eq!(filter_res.matches.len(), 3);
    assert!(!filter_res.matches.iter().any(|m| m.path.ends_with(".png")));
}

#[test]
fn test_search_text_reauthorization_revocation() {
    let env = TestEnv::new("search_text_reauth");
    let cfg = env.config_dir();
    let root = env.new_dir("TextReauthRoot");
    let canon_root = dunce::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .to_string();

    add_project_root_path(&cfg, &root).unwrap();
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(true)).unwrap();

    let file = root.join("source.cs");
    std::fs::write(&file, b"public void DoAction() { Log(\"SecretValue\"); }\n").unwrap();

    let mut res_str = execute_tool(
        &cfg,
        "search_project_text",
        &serde_json::json!({
            "query": "SecretValue",
            "project_root": canon_root
        })
        .to_string(),
    );
    assert!(res_str.contains("SecretValue"));

    // Revoke sharing
    toggle_project_provider_sharing_impl(&cfg, &canon_root, Some(false)).unwrap();

    // Reauthorize before transmission
    reauthorize_search_text_result(&cfg, &mut res_str);
    assert!(res_str.contains("error"), "Must replace result with error");
    assert!(
        !res_str.contains("SecretValue"),
        "No text snippets or secret paths may leak"
    );
    assert!(res_str.contains("revoked_grant"));
}
