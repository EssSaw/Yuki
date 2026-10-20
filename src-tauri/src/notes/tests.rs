use std::path::PathBuf;

use super::models::{
    CURRENT_NOTE_SCHEMA_VERSION, DiskNoteRecord, MAX_NOTE_BODY_BYTES, MAX_NOTE_TITLE_CHARS,
    NoteError,
};
use super::storage::{
    check_no_symlink_or_reparse, compute_version_token, create_note_impl,
    get_data_root_from_config, list_notes_impl, load_note_impl, resolve_notes_dir, save_note_impl,
    validate_data_root, validate_note_id,
};
use crate::workspace::models::WorkspaceSettings;
use crate::workspace::settings::save_settings_atomic;

/// Test helper providing an isolated sandbox within the project directory (target/test_temp),
/// strictly avoiding /tmp, /var/tmp, or user directories.
struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(name: &str) -> Self {
        let unique = format!(
            "notes_{}_{}_{}",
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

    fn data_root(&self) -> PathBuf {
        let dir = self.root.join("data_root");
        std::fs::create_dir_all(&dir).unwrap();
        dunce::canonicalize(&dir).unwrap()
    }

    fn config_dir(&self) -> PathBuf {
        let dir = self.root.join("config");
        std::fs::create_dir_all(&dir).unwrap();
        dunce::canonicalize(&dir).unwrap()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_uuid_validation_and_rejections() {
    // Valid v4 random UUID
    let valid_v4 = uuid::Uuid::new_v4().to_string();
    let res = validate_note_id(&valid_v4);
    assert!(res.is_ok());

    // Non-UUID string
    assert!(matches!(
        validate_note_id("not-a-valid-uuid"),
        Err(NoteError::InvalidUuid(_))
    ));

    // Path traversal attempt
    assert!(matches!(
        validate_note_id("../../etc/passwd"),
        Err(NoteError::InvalidUuid(_))
    ));
    assert!(matches!(
        validate_note_id("..\\..\\windows\\system32"),
        Err(NoteError::InvalidUuid(_))
    ));

    // DOS device names
    assert!(matches!(
        validate_note_id("CON"),
        Err(NoteError::InvalidUuid(_))
    ));

    // Empty string
    assert!(matches!(
        validate_note_id(""),
        Err(NoteError::InvalidUuid(_))
    ));

    // Uppercase UUID (must be canonical lowercase)
    let uppercase_v4 = valid_v4.to_ascii_uppercase();
    assert!(matches!(
        validate_note_id(&uppercase_v4),
        Err(NoteError::InvalidUuid(_))
    ));

    // UUID v1 (reject non-v4)
    // UUID v1 has version nibble 1 in time_hi_and_version
    let v1_str = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
    assert!(matches!(
        validate_note_id(v1_str),
        Err(NoteError::InvalidUuid(_))
    ));
}

#[test]
fn test_path_validation_and_ancestors() {
    let env = TestEnv::new("path_val");
    let valid_root = env.data_root();

    // Valid directory passes
    let validated = validate_data_root(&valid_root);
    assert!(validated.is_ok());

    // Nonexistent directory
    let nonexistent = valid_root.join("does_not_exist");
    assert!(matches!(
        validate_data_root(&nonexistent),
        Err(NoteError::DataRootNotFound(_))
    ));

    // Regular file instead of directory
    let file_path = valid_root.join("some_file.txt");
    std::fs::write(&file_path, b"test").unwrap();
    assert!(matches!(
        validate_data_root(&file_path),
        Err(NoteError::DataRootInvalid(_))
    ));

    // UNC or device path rejected
    let unc_path = PathBuf::from(r"\\server\share\data");
    assert!(matches!(
        validate_data_root(&unc_path),
        Err(NoteError::SecurityViolation(_))
    ));
}

#[test]
fn test_symlink_or_reparse_check() {
    let env = TestEnv::new("symlink_check");
    let regular_file = env.root.join("regular.txt");
    std::fs::write(&regular_file, b"content").unwrap();

    assert!(check_no_symlink_or_reparse(&regular_file).is_ok());

    let non_existent = env.root.join("non_existent.txt");
    assert!(check_no_symlink_or_reparse(&non_existent).is_ok());
}

#[test]
fn test_missing_root_config() {
    let env = TestEnv::new("missing_root");
    let cfg = env.config_dir();

    // No settings.json exists
    let res = get_data_root_from_config(&cfg);
    assert!(matches!(res, Err(NoteError::DataRootNotConfigured(_))));

    // settings.json exists with data_root = None
    let settings = WorkspaceSettings::default();
    save_settings_atomic(&cfg, &settings).unwrap();
    let res2 = get_data_root_from_config(&cfg);
    assert!(matches!(res2, Err(NoteError::DataRootNotConfigured(_))));

    // settings.json exists with data_root = Some("")
    let mut empty_root_settings = WorkspaceSettings::default();
    empty_root_settings.data_root = Some("   ".to_string());
    save_settings_atomic(&cfg, &empty_root_settings).unwrap();
    let res3 = get_data_root_from_config(&cfg);
    assert!(matches!(res3, Err(NoteError::DataRootNotConfigured(_))));
}

#[test]
fn test_create_and_load_success() {
    let env = TestEnv::new("create_load");
    let data_root = env.data_root();

    let title = "Meeting Summary";
    let body = "# Meeting Notes\n\n- Discussed native storage\n- Architecture verified";

    // Create note
    let created = create_note_impl(&data_root, title, body).unwrap();
    assert_eq!(created.schema_version, CURRENT_NOTE_SCHEMA_VERSION);
    assert_eq!(created.title, title);
    assert_eq!(created.body, body);
    assert!(!created.id.is_empty());
    assert!(!created.version_token.is_empty());

    // File exists on disk at notes/<uuid>.json
    let notes_dir = resolve_notes_dir(&data_root).unwrap();
    let note_file = notes_dir.join(format!("{}.json", created.id));
    assert!(note_file.exists());

    // Load note
    let loaded = load_note_impl(&data_root, &created.id).unwrap();
    assert_eq!(loaded.id, created.id);
    assert_eq!(loaded.title, title);
    assert_eq!(loaded.body, body);
    assert_eq!(loaded.created_at, created.created_at);
    assert_eq!(loaded.updated_at, created.updated_at);
    assert_eq!(loaded.version_token, created.version_token);
}

#[test]
fn test_rename_stable_id() {
    let env = TestEnv::new("rename_stable_id");
    let data_root = env.data_root();

    let initial = create_note_impl(&data_root, "Initial Title", "# Content").unwrap();
    let original_id = initial.id.clone();
    let original_created_at = initial.created_at.clone();

    // Rename title and update body
    let renamed = save_note_impl(
        &data_root,
        &original_id,
        "Renamed Title",
        "# Updated Content",
        &initial.version_token,
    )
    .unwrap();

    // ID must remain completely stable
    assert_eq!(renamed.id, original_id);
    assert_eq!(renamed.title, "Renamed Title");
    assert_eq!(renamed.body, "# Updated Content");
    assert_eq!(renamed.created_at, original_created_at);
    assert_ne!(renamed.version_token, initial.version_token);

    // Verify on disk: file name is still <original_id>.json
    let notes_dir = resolve_notes_dir(&data_root).unwrap();
    let expected_file = notes_dir.join(format!("{}.json", original_id));
    assert!(expected_file.exists());

    let loaded = load_note_impl(&data_root, &original_id).unwrap();
    assert_eq!(loaded.title, "Renamed Title");
    assert_eq!(loaded.id, original_id);
}

#[test]
fn test_optimistic_conflict_and_external_edits() {
    let env = TestEnv::new("conflict_external");
    let data_root = env.data_root();

    let note = create_note_impl(&data_root, "Version Test", "Initial body").unwrap();

    // 1. Save with outdated version token must fail with VersionConflict
    let conflict_res = save_note_impl(
        &data_root,
        &note.id,
        "Updated Title",
        "Updated body",
        "stale_version_token:123",
    );
    assert!(matches!(
        conflict_res,
        Err(NoteError::VersionConflict { .. })
    ));

    // 2. Simulate external edit: modifying raw bytes on disk
    let notes_dir = resolve_notes_dir(&data_root).unwrap();
    let file_path = notes_dir.join(format!("{}.json", note.id));
    let mut raw_bytes = std::fs::read(&file_path).unwrap();
    // Invert one byte in the JSON
    raw_bytes.push(b' ');
    std::fs::write(&file_path, &raw_bytes).unwrap();

    // Attempting to save with original version token must fail because bytes changed externally!
    let ext_conflict_res = save_note_impl(
        &data_root,
        &note.id,
        "Updated Title",
        "Updated body",
        &note.version_token,
    );
    assert!(matches!(
        ext_conflict_res,
        Err(NoteError::VersionConflict { .. })
    ));

    // Reload note gets updated version token matching new on-disk bytes
    let reloaded = load_note_impl(&data_root, &note.id).unwrap();
    assert_ne!(reloaded.version_token, note.version_token);

    // Save with fresh version token succeeds
    let saved = save_note_impl(
        &data_root,
        &note.id,
        "Finally Saved",
        "Fresh content",
        &reloaded.version_token,
    )
    .unwrap();
    assert_eq!(saved.title, "Finally Saved");
}

#[test]
fn test_unicode_and_newlines() {
    let env = TestEnv::new("unicode_newlines");
    let data_root = env.data_root();

    let unicode_title = "Ghi chú Yuki ❄️ 日本語 — Accents: café & crème";
    let multiline_body = "# Heading\r\n\r\nLine 1 with CRLF\r\nLine 2 with LF\n\n```rust\nfn main() {\n    println!(\"Hello Yuki\");\n}\n```\n\n- Item 1 ☕\n- Item 2 🌸\n";

    let created = create_note_impl(&data_root, unicode_title, multiline_body).unwrap();
    assert_eq!(created.title, unicode_title);
    assert_eq!(created.body, multiline_body);

    let loaded = load_note_impl(&data_root, &created.id).unwrap();
    assert_eq!(loaded.title, unicode_title);
    assert_eq!(loaded.body, multiline_body);
}

#[test]
fn test_title_and_payload_limits() {
    let env = TestEnv::new("limits");
    let data_root = env.data_root();

    // Title limit: 200 chars allowed, 201 chars rejected
    let title_200 = "a".repeat(MAX_NOTE_TITLE_CHARS);
    let note = create_note_impl(&data_root, &title_200, "Normal body").unwrap();
    assert_eq!(note.title.len(), MAX_NOTE_TITLE_CHARS);

    let title_201 = "a".repeat(MAX_NOTE_TITLE_CHARS + 1);
    let err_title = create_note_impl(&data_root, &title_201, "Normal body");
    assert!(matches!(
        err_title,
        Err(NoteError::TitleTooLong {
            limit: 200,
            actual: 201
        })
    ));

    // Body limit: 4 MiB allowed, 4 MiB + 1 byte rejected
    let body_oversized = "x".repeat(MAX_NOTE_BODY_BYTES + 1);
    let err_body = create_note_impl(&data_root, "Short title", &body_oversized);
    assert!(matches!(
        err_body,
        Err(NoteError::PayloadTooLarge {
            limit: 4194304,
            actual: 4194305
        })
    ));

    // Save rejection on limit preserves old note on disk
    let err_save = save_note_impl(
        &data_root,
        &note.id,
        &title_201,
        "Normal body",
        &note.version_token,
    );
    assert!(matches!(err_save, Err(NoteError::TitleTooLong { .. })));

    let reloaded = load_note_impl(&data_root, &note.id).unwrap();
    assert_eq!(reloaded.title, title_200);
}

#[test]
fn test_corrupt_data_never_overwritten() {
    let env = TestEnv::new("corrupt_protection");
    let data_root = env.data_root();

    let note = create_note_impl(&data_root, "Valid Note", "Valid body").unwrap();
    let notes_dir = resolve_notes_dir(&data_root).unwrap();
    let file_path = notes_dir.join(format!("{}.json", note.id));

    // Corrupt the file on disk
    let corrupt_bytes = b"CORRUPTED INVALID JSON {{{ not json";
    std::fs::write(&file_path, corrupt_bytes).unwrap();

    // Loading corrupt note must return CorruptData
    let load_res = load_note_impl(&data_root, &note.id);
    assert!(matches!(load_res, Err(NoteError::CorruptData(_))));

    // Attempting to save over a corrupt file must NEVER overwrite it!
    let save_res = save_note_impl(
        &data_root,
        &note.id,
        "Overwriting Attempt",
        "New Body",
        &note.version_token,
    );
    assert!(matches!(save_res, Err(NoteError::CorruptData(_))));

    // Verify corrupt file was preserved without modification
    let current_bytes = std::fs::read(&file_path).unwrap();
    assert_eq!(current_bytes, corrupt_bytes);
}

#[test]
fn test_future_schema_version_never_overwritten() {
    let env = TestEnv::new("future_schema");
    let data_root = env.data_root();
    let notes_dir = resolve_notes_dir(&data_root).unwrap();

    let future_uuid = uuid::Uuid::new_v4();
    let file_path = notes_dir.join(format!("{}.json", future_uuid));

    let future_record = DiskNoteRecord {
        schema_version: 999, // Future version
        id: future_uuid.to_string(),
        title: "Future Note".to_string(),
        body: "From Yuki v10".to_string(),
        created_at: "2030-01-01T00:00:00Z".to_string(),
        updated_at: "2030-01-01T00:00:00Z".to_string(),
    };
    let json_bytes = serde_json::to_vec_pretty(&future_record).unwrap();
    std::fs::write(&file_path, &json_bytes).unwrap();

    // Loading future schema version returns FutureSchemaVersion
    let load_res = load_note_impl(&data_root, &future_uuid.to_string());
    assert!(matches!(
        load_res,
        Err(NoteError::FutureSchemaVersion {
            expected: 1,
            actual: 999
        })
    ));

    let token = compute_version_token(&json_bytes);

    // Saving over future schema version is rejected; file preserved
    let save_res = save_note_impl(
        &data_root,
        &future_uuid.to_string(),
        "Downgrade Attempt",
        "Body",
        &token,
    );
    assert!(matches!(
        save_res,
        Err(NoteError::FutureSchemaVersion {
            expected: 1,
            actual: 999
        })
    ));

    let current_bytes = std::fs::read(&file_path).unwrap();
    assert_eq!(current_bytes, json_bytes);
}

#[test]
fn test_backup_recovery_conservative() {
    let env = TestEnv::new("backup_recovery");
    let data_root = env.data_root();
    let notes_dir = resolve_notes_dir(&data_root).unwrap();

    let uuid = uuid::Uuid::new_v4();
    let dest_file = notes_dir.join(format!("{}.json", uuid));
    let backup_file = notes_dir.join(format!(".{}.json.bak_9999_123456", uuid));

    // Primary file missing, single valid backup file exists
    let backup_record = DiskNoteRecord {
        schema_version: 1,
        id: uuid.to_string(),
        title: "Recovered Note".to_string(),
        body: "Recovered body from backup".to_string(),
        created_at: "2026-10-20T12:00:00Z".to_string(),
        updated_at: "2026-10-20T12:00:00Z".to_string(),
    };
    let json_bytes = serde_json::to_vec_pretty(&backup_record).unwrap();
    std::fs::write(&backup_file, &json_bytes).unwrap();

    assert!(!dest_file.exists());
    assert!(backup_file.exists());

    // load_note_impl recovers the file
    let loaded = load_note_impl(&data_root, &uuid.to_string()).unwrap();
    assert_eq!(loaded.id, uuid.to_string());
    assert_eq!(loaded.title, "Recovered Note");
    assert!(dest_file.exists());
}

#[test]
fn test_ambiguous_recovery_preserves_files() {
    let env = TestEnv::new("ambiguous_recovery");
    let data_root = env.data_root();
    let notes_dir = resolve_notes_dir(&data_root).unwrap();

    let uuid = uuid::Uuid::new_v4();
    let backup_1 = notes_dir.join(format!(".{}.json.bak_1_111", uuid));
    let backup_2 = notes_dir.join(format!(".{}.json.bak_2_222", uuid));

    let record1 = DiskNoteRecord {
        schema_version: 1,
        id: uuid.to_string(),
        title: "Backup 1".to_string(),
        body: "Body 1".to_string(),
        created_at: "2026-10-20T12:00:00Z".to_string(),
        updated_at: "2026-10-20T12:00:00Z".to_string(),
    };
    let record2 = DiskNoteRecord {
        schema_version: 1,
        id: uuid.to_string(),
        title: "Backup 2".to_string(),
        body: "Body 2".to_string(),
        created_at: "2026-10-20T12:05:00Z".to_string(),
        updated_at: "2026-10-20T12:05:00Z".to_string(),
    };

    std::fs::write(&backup_1, serde_json::to_vec_pretty(&record1).unwrap()).unwrap();
    std::fs::write(&backup_2, serde_json::to_vec_pretty(&record2).unwrap()).unwrap();

    // Recovery must fail with AmbiguousRecovery
    let res = load_note_impl(&data_root, &uuid.to_string());
    assert!(matches!(res, Err(NoteError::AmbiguousRecovery(_))));

    // Both files must be preserved!
    assert!(backup_1.exists());
    assert!(backup_2.exists());
}

#[test]
fn test_failed_save_retains_old_note() {
    let env = TestEnv::new("failure_retention");
    let data_root = env.data_root();

    let note = create_note_impl(&data_root, "Safe Title", "Safe Content").unwrap();

    // Trigger save conflict
    let err = save_note_impl(
        &data_root,
        &note.id,
        "Bad Save",
        "Bad Content",
        "wrong_token",
    );
    assert!(err.is_err());

    // Verify existing note was retained completely intact
    let loaded = load_note_impl(&data_root, &note.id).unwrap();
    assert_eq!(loaded.title, "Safe Title");
    assert_eq!(loaded.body, "Safe Content");
}

#[test]
fn test_list_notes_enumeration_and_order() {
    let env = TestEnv::new("list_notes");
    let data_root = env.data_root();

    // Initially empty
    let initial_list = list_notes_impl(&data_root).unwrap();
    assert!(initial_list.is_empty());

    // Create 3 notes
    let n1 = create_note_impl(&data_root, "Note 1", "Body 1").unwrap();
    let n2 = create_note_impl(&data_root, "Note 2", "Body 2").unwrap();
    let n3 = create_note_impl(&data_root, "Note 3", "Body 3").unwrap();

    let listed = list_notes_impl(&data_root).unwrap();
    assert_eq!(listed.len(), 3);

    let ids: Vec<_> = listed.iter().map(|n| n.id.clone()).collect();
    assert!(ids.contains(&n1.id));
    assert!(ids.contains(&n2.id));
    assert!(ids.contains(&n3.id));
}
