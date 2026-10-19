use std::path::{Path, PathBuf};

use super::explorer::{
    is_blocked_name, is_drive_root, is_system_directory, is_user_profile_root,
    list_project_directory_impl, validate_project_root,
};
use super::models::WorkspaceSettings;
use super::settings::{
    add_project_root_path, choose_data_root_path, get_workspace_settings_impl, load_settings,
    remove_project_root_impl, resolve_data_root_subdir, save_settings_atomic,
};

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
