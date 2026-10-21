use super::*;

struct Fixture {
    base: PathBuf,
    config: PathBuf,
    project: PathBuf,
    data: PathBuf,
    file: PathBuf,
    conversation: String,
    generation: u64,
    edits: EditProposals,
}
impl Fixture {
    fn new() -> Self {
        use crate::workspace::settings::*;
        let base = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("edit-recovery-{}", Uuid::new_v4()));
        let config = base.join("config");
        let project = base.join("project");
        let data = base.join("data");
        for dir in [&config, &project, &data] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let project = dunce::canonicalize(project).unwrap();
        let mut settings = add_project_root_path(&config, &project).unwrap();
        settings.data_root = Some(data.to_string_lossy().into());
        save_settings_atomic(&config, &settings).unwrap();
        let grant = crate::workspace::agent::approve_agent_write_root_impl(
            &config,
            &project.to_string_lossy(),
            &project.to_string_lossy(),
            Some(settings.grant_generation),
        )
        .unwrap();
        let conversation = Uuid::new_v4().to_string();
        let mut record =
            crate::chat::models::ConversationDetail::new(&conversation, "test", "now", vec![]);
        record.mode = ConversationMode::Agent;
        record.task_root = Some(project.to_string_lossy().into());
        record.task_revision = Uuid::new_v4().to_string();
        crate::chat::history::save_conversation_atomic(&data, &record).unwrap();
        let file = project.join("file.txt");
        std::fs::write(&file, "original").unwrap();
        Self {
            base,
            config,
            project,
            data,
            file,
            conversation,
            generation: grant.snapshot.generation,
            edits: EditProposals::default(),
        }
    }
    fn prepare(&self) -> EditProposalSummary {
        self.edits
            .prepare(
                &self.config,
                &self.conversation,
                &self.file.to_string_lossy(),
                self.generation,
                "replacement",
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[test]
fn unavailable_recovery_leaves_original_untouched() {
    let f = Fixture::new();
    let proposal = f.prepare();
    std::fs::write(f.data.join("edit-recovery"), "blocks directory").unwrap();
    assert!(f.edits.apply(&f.config, &proposal.id).is_err());
    assert_eq!(std::fs::read(&f.file).unwrap(), b"original");
}

#[test]
fn hard_link_targets_cannot_be_mutated() {
    let f = Fixture::new();
    let proposal = f.prepare();
    let alias = f.project.join("alias.txt");
    std::fs::hard_link(&f.file, &alias).unwrap();
    assert!(
        f.edits
            .apply(&f.config, &proposal.id)
            .unwrap_err()
            .contains("Hard-linked")
    );
    assert_eq!(std::fs::read(alias).unwrap(), b"original");
}

#[test]
fn held_target_excludes_competing_writes_and_directory_rename() {
    let f = Fixture::new();
    let _held = target::LockedTarget::open(&f.file).unwrap();
    assert!(
        std::fs::OpenOptions::new()
            .write(true)
            .open(&f.file)
            .is_err()
    );
    assert!(std::fs::rename(&f.project, f.base.join("renamed")).is_err());
}

#[test]
fn corrupt_commit_record_never_overwrites_target() {
    let f = Fixture::new();
    let proposal = f.prepare();
    let id = f.edits.apply(&f.config, &proposal.id).unwrap();
    std::fs::write(
        f.data
            .join("edit-recovery")
            .join(format!("{id}.committed.json")),
        "invalid",
    )
    .unwrap();
    assert!(f.edits.undo(&f.config, &id, f.generation).is_err());
    assert_eq!(std::fs::read(&f.file).unwrap(), b"replacement");
}
#[test]
fn cancelled_edit_never_changes_file_and_uncertain_records_are_reported() {
    let f = Fixture::new();
    let proposal = f.prepare();
    assert!(
        f.edits
            .apply_checked(&f.config, &proposal.id, || true)
            .is_err()
    );
    assert_eq!(std::fs::read(&f.file).unwrap(), b"original");
    let id = f.edits.apply(&f.config, &proposal.id).unwrap();
    let committed = f
        .data
        .join("edit-recovery")
        .join(format!("{id}.committed.json"));
    std::fs::remove_file(committed).unwrap();
    let inventory = recovery::inventory(&f.config).unwrap();
    assert_eq!(inventory[0].state, "uncertainApply");
    assert!(inventory[0].review_required);
    assert_eq!(std::fs::read(&f.file).unwrap(), b"replacement");
}
#[test]
fn inventory_rejects_corrupt_or_disagreeing_stage_content() {
    let f = Fixture::new();
    let proposal = f.prepare();
    let id = f.edits.apply(&f.config, &proposal.id).unwrap();
    assert_eq!(
        recovery::inventory(&f.config).unwrap()[0].state,
        "committed"
    );
    let path = f
        .data
        .join("edit-recovery")
        .join(format!("{id}.committed.json"));
    let mut record: recovery::ChangeRecord =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record.after = b"tampered".to_vec();
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(recovery::inventory(&f.config).is_err());
    assert!(f.edits.undo(&f.config, &id, f.generation).is_err());
    assert_eq!(std::fs::read(&f.file).unwrap(), b"replacement");
    std::fs::write(path, b"corrupt").unwrap();
    assert!(recovery::inventory(&f.config).is_err());
}

#[test]
fn recovery_quota_is_reserved_before_target_mutation() {
    let f = Fixture::new();
    // JSON byte arrays consume roughly 8 MiB per before/after pair here.
    // Four-stage reservations must reject well before disk usage reaches 64 MiB.
    let bytes = vec![b'x'; MAX_EDIT_BYTES];
    std::fs::write(&f.file, &bytes).unwrap();
    let replacement = "y".repeat(MAX_EDIT_BYTES);
    let proposal = f
        .edits
        .prepare(
            &f.config,
            &f.conversation,
            &f.file.to_string_lossy(),
            f.generation,
            &replacement,
        )
        .unwrap();
    f.edits.apply(&f.config, &proposal.id).unwrap();
    let other = f.project.join("other.txt");
    std::fs::write(&other, &bytes).unwrap();
    let proposal = f
        .edits
        .prepare(
            &f.config,
            &f.conversation,
            &other.to_string_lossy(),
            f.generation,
            &replacement,
        )
        .unwrap();
    assert!(
        f.edits
            .apply(&f.config, &proposal.id)
            .unwrap_err()
            .contains("quota")
    );
    assert_eq!(std::fs::read(other).unwrap(), bytes);
}
