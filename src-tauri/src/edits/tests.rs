use super::*;
use crate::chat::history::save_conversation_atomic;
use crate::chat::models::ConversationDetail;
use crate::workspace::agent::approve_agent_write_root_impl;
use crate::workspace::settings::{add_project_root_path, load_settings, save_settings_atomic};

#[test]
fn proposals_preserve_files_and_detect_conflicts_and_revocation() {
    let base = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("edit-test-{}", Uuid::new_v4()));
    let config = base.join("config");
    let project = base.join("project");
    let data = base.join("data");
    for path in [&config, &project, &data] {
        std::fs::create_dir_all(path).unwrap();
    }
    let project = dunce::canonicalize(project).unwrap();
    let target = project.join("example.txt");
    std::fs::write(&target, "old\r\n").unwrap();
    let mut settings = add_project_root_path(&config, &project).unwrap();
    settings.data_root = Some(data.to_string_lossy().into());
    save_settings_atomic(&config, &settings).unwrap();
    let grant = approve_agent_write_root_impl(
        &config,
        &project.to_string_lossy(),
        &project.to_string_lossy(),
        Some(settings.grant_generation),
    )
    .unwrap();
    let id = Uuid::new_v4().to_string();
    let mut conversation = ConversationDetail::new(&id, "test", "now", vec![]);
    conversation.mode = ConversationMode::Agent;
    conversation.task_root = Some(project.to_string_lossy().into());
    save_conversation_atomic(&data, &conversation).unwrap();
    let proposals = EditProposals::default();
    assert!(
        proposals
            .prepare(
                &config,
                &id,
                &target.to_string_lossy(),
                grant.snapshot.generation,
                "new\n"
            )
            .is_err()
    );
    let proposal = proposals
        .prepare(
            &config,
            &id,
            &target.to_string_lossy(),
            grant.snapshot.generation,
            "new\r\n",
        )
        .unwrap();
    assert_eq!(proposal.apply_available, cfg!(windows));
    assert_eq!(std::fs::read(&target).unwrap(), b"old\r\n");
    proposals.revalidate(&config, &proposal.id).unwrap();
    let change = proposals.apply(&config, &proposal.id).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"new\r\n");
    std::fs::write(&target, "external after apply\r\n").unwrap();
    assert!(
        proposals
            .undo(&config, &change, grant.snapshot.generation)
            .is_err()
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"external after apply\r\n");
    std::fs::write(&target, "new\r\n").unwrap();
    proposals
        .undo(&config, &change, grant.snapshot.generation)
        .unwrap();
    assert!(
        proposals
            .undo(&config, &change, grant.snapshot.generation)
            .is_err()
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"old\r\n");
    let proposal = proposals
        .prepare(
            &config,
            &id,
            &target.to_string_lossy(),
            grant.snapshot.generation,
            "new\r\n",
        )
        .unwrap();
    conversation.task_revision = Uuid::new_v4().to_string();
    save_conversation_atomic(&data, &conversation).unwrap();
    assert!(proposals.apply(&config, &proposal.id).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"old\r\n");
    proposals.discard(&proposal.id).unwrap();
    let proposal = proposals
        .prepare(
            &config,
            &id,
            &target.to_string_lossy(),
            grant.snapshot.generation,
            "new\r\n",
        )
        .unwrap();
    std::fs::write(&target, "external\r\n").unwrap();
    assert!(
        proposals
            .revalidate(&config, &proposal.id)
            .unwrap_err()
            .contains("conflicts")
    );
    std::fs::write(&target, "old\r\n").unwrap();
    let mut settings = load_settings(&config).unwrap();
    settings.grant_generation += 1;
    save_settings_atomic(&config, &settings).unwrap();
    assert!(proposals.revalidate(&config, &proposal.id).is_err());
    proposals.discard(&proposal.id).unwrap();
    assert!(proposals.revalidate(&config, &proposal.id).is_err());
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn unsupported_text_and_stream_paths_are_rejected() {
    assert!(validate_text(b"a\0b").is_err());
    assert!(validate_text(&[0xff]).is_err());
    assert!(validate_text(&vec![b'a'; MAX_EDIT_BYTES + 1]).is_err());
    assert!(restricted_stream_path(Path::new("file.txt:stream")));
}
