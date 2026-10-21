pub mod commands;
pub mod recovery;
mod target;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use uuid::Uuid;

use crate::chat::history::load_conversation_from_disk;
use crate::chat::models::ConversationMode;
use crate::workspace::agent::validate_write_root;
use crate::workspace::explorer::validate_read_path;
use crate::workspace::settings::load_settings;

pub const MAX_EDIT_BYTES: usize = 1024 * 1024;
const MAX_PROPOSALS: usize = 8;
const PROPOSAL_LIFETIME: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditProposalSummary {
    pub id: String,
    pub conversation_id: String,
    pub path: String,
    pub grant_id: String,
    pub generation: u64,
    pub original_bytes: usize,
    pub replacement_bytes: usize,
    pub apply_available: bool,
}

struct Proposal {
    summary: EditProposalSummary,
    original: Vec<u8>,
    replacement: Vec<u8>,
    task_root: String,
    task_description: Option<String>,
    task_revision: String,
    created: Instant,
}

#[derive(Default)]
pub struct EditProposals {
    proposals: Mutex<HashMap<String, Proposal>>,
}

fn restricted_stream_path(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(component, std::path::Component::Normal(name) if name.to_string_lossy().contains(':'))
    })
}

fn read_text(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|_| "Cannot open edit target")?;
    if file
        .metadata()
        .map_err(|_| "Cannot inspect edit target")?
        .len()
        > MAX_EDIT_BYTES as u64
    {
        return Err(
            "This editor supports existing text files up to 1 MiB; file left unchanged".into(),
        );
    }
    let mut bytes = Vec::new();
    file.take((MAX_EDIT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read edit target")?;
    validate_text(&bytes)?;
    Ok(bytes)
}

fn validate_text(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_EDIT_BYTES {
        return Err("This editor supports text up to 1 MiB; file left unchanged".into());
    }
    if bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
        return Err("Only UTF-8 text without NUL bytes is supported".into());
    }
    Ok(())
}

struct Authorization {
    path: PathBuf,
    task_root: String,
    task_description: Option<String>,
    grant_id: String,
    task_revision: String,
}

fn authorize(
    config: &Path,
    conversation_id: &str,
    path: &str,
    generation: u64,
) -> Result<Authorization, String> {
    let settings = load_settings(config)?;
    if settings.grant_generation != generation {
        return Err("Stale Agent grant generation; refresh permissions".into());
    }
    let data_root = settings
        .data_root
        .as_ref()
        .ok_or("Choose a data root first")?;
    let conversation = load_conversation_from_disk(Path::new(data_root), conversation_id)?;
    if conversation.mode != ConversationMode::Agent {
        return Err("Text edit proposals require explicit Agent mode".into());
    }
    let task_root = conversation
        .task_root
        .ok_or("Agent task has no project root")?;
    let project = dunce::canonicalize(&task_root).map_err(|_| "Task project is unavailable")?;
    if !settings
        .project_roots
        .iter()
        .any(|root| dunce::canonicalize(root).ok().as_ref() == Some(&project))
    {
        return Err("Task project approval was revoked".into());
    }
    if restricted_stream_path(Path::new(path)) {
        return Err("Alternate file streams are not supported".into());
    }
    let target = validate_read_path(config, path)?;
    let parent = target.parent().ok_or("Invalid edit target")?;
    validate_write_root(config, &project, &parent.to_string_lossy())?;
    let grant = settings
        .agent_write_roots
        .iter()
        .find(|grant| {
            dunce::canonicalize(&grant.project_root).ok().as_ref() == Some(&project)
                && validate_write_root(config, &project, &grant.canonical_path)
                    .map(|root| target.starts_with(root))
                    .unwrap_or(false)
        })
        .ok_or("No current write-root grant covers this file")?;
    Ok(Authorization {
        path: target,
        task_root,
        task_description: conversation.task_description,
        grant_id: grant.id.clone(),
        task_revision: conversation.task_revision,
    })
}

impl EditProposals {
    pub fn prepare(
        &self,
        config: &Path,
        conversation_id: &str,
        path: &str,
        generation: u64,
        replacement: &str,
    ) -> Result<EditProposalSummary, String> {
        validate_text(replacement.as_bytes())?;
        let authorization = authorize(config, conversation_id, path, generation)?;
        let original = read_text(&authorization.path)?;
        // Full replacements must preserve BOM presence and newline convention.
        if original.starts_with(&[0xef, 0xbb, 0xbf])
            != replacement.as_bytes().starts_with(&[0xef, 0xbb, 0xbf])
        {
            return Err("Replacement must preserve the original UTF-8 BOM".into());
        }
        if original.windows(2).any(|pair| pair == b"\r\n")
            && replacement.as_bytes().iter().enumerate().any(|(i, byte)| {
                *byte == b'\n' && (i == 0 || replacement.as_bytes()[i - 1] != b'\r')
            })
        {
            return Err("Replacement must preserve CRLF line endings".into());
        }
        let mut proposals = self
            .proposals
            .lock()
            .map_err(|_| "Proposal state unavailable")?;
        proposals.retain(|_, proposal| proposal.created.elapsed() < PROPOSAL_LIFETIME);
        if proposals.len() >= MAX_PROPOSALS {
            return Err(
                "Pending proposal limit reached; discard a proposal before preparing another"
                    .into(),
            );
        }
        let summary = EditProposalSummary {
            id: Uuid::new_v4().to_string(),
            conversation_id: conversation_id.into(),
            path: authorization.path.to_string_lossy().into(),
            grant_id: authorization.grant_id,
            generation,
            original_bytes: original.len(),
            replacement_bytes: replacement.len(),
            apply_available: cfg!(windows),
        };
        proposals.insert(
            summary.id.clone(),
            Proposal {
                summary: summary.clone(),
                original,
                replacement: replacement.as_bytes().into(),
                task_root: authorization.task_root,
                task_description: authorization.task_description,
                task_revision: authorization.task_revision,
                created: Instant::now(),
            },
        );
        Ok(summary)
    }

    pub fn revalidate(
        &self,
        config: &Path,
        proposal_id: &str,
    ) -> Result<EditProposalSummary, String> {
        let proposals = self
            .proposals
            .lock()
            .map_err(|_| "Proposal state unavailable")?;
        let proposal = proposals.get(proposal_id).ok_or("Proposal not found")?;
        if proposal.created.elapsed() >= PROPOSAL_LIFETIME {
            return Err("Proposal expired".into());
        }
        let summary = &proposal.summary;
        let authorization = authorize(
            config,
            &summary.conversation_id,
            &summary.path,
            summary.generation,
        )?;
        if authorization.task_revision != proposal.task_revision
            || authorization.grant_id != summary.grant_id
            || authorization.task_root != proposal.task_root
            || authorization.task_description != proposal.task_description
        {
            return Err("Agent task or grant changed; prepare a new proposal".into());
        }
        if read_text(&authorization.path)? != proposal.original {
            return Err("File changed externally; proposal conflicts".into());
        }
        validate_text(&proposal.replacement)?;
        Ok(summary.clone())
    }

    pub fn apply(&self, config: &Path, proposal_id: &str) -> Result<String, String> {
        self.apply_checked(config, proposal_id, || false)
    }

    pub fn apply_checked(
        &self,
        config: &Path,
        proposal_id: &str,
        cancelled: impl Fn() -> bool,
    ) -> Result<String, String> {
        if cancelled() {
            return Err("Edit cancelled before mutation".into());
        }
        let _settings = crate::workspace::settings::SETTINGS_MUTATION_LOCK
            .lock()
            .map_err(|_| "Settings unavailable")?;
        self.revalidate(config, proposal_id)?;
        let mut proposals = self
            .proposals
            .lock()
            .map_err(|_| "Proposal state unavailable")?;
        let proposal = proposals.get(proposal_id).ok_or("Proposal unavailable")?;
        let mut target = target::LockedTarget::open(Path::new(&proposal.summary.path))?;
        if target.read()? != proposal.original {
            return Err("File changed externally; left unchanged".into());
        }
        let mut record = recovery::ChangeRecord {
            schema_version: 1,
            id: Uuid::new_v4().to_string(),
            conversation_id: proposal.summary.conversation_id.clone(),
            path: proposal.summary.path.clone(),
            task_root: proposal.task_root.clone(),
            task_revision: proposal.task_revision.clone(),
            grant_id: proposal.summary.grant_id.clone(),
            state: "prepared".into(),
            before: proposal.original.clone(),
            after: proposal.replacement.clone(),
        };
        if cancelled() {
            return Err("Edit cancelled before mutation".into());
        }
        recovery::append(config, &record)?;
        if cancelled() {
            record.state = "cancelled".into();
            recovery::append(config, &record)?;
            return Err("Edit cancelled before mutation; recovery retained".into());
        }
        // A prepared record is never replayed automatically after interruption.
        let write_result = target.write(&record.after);
        proposals.remove(proposal_id);
        write_result?;
        record.state = "committed".into();
        recovery::append(config, &record).map_err(|_| {
            "File changed but commit record failed; outcome requires recovery review".to_string()
        })?;
        Ok(record.id)
    }

    pub fn undo(&self, config: &Path, change_id: &str, generation: u64) -> Result<(), String> {
        self.undo_checked(config, change_id, generation, || false)
    }

    pub fn undo_checked(
        &self,
        config: &Path,
        change_id: &str,
        generation: u64,
        cancelled: impl Fn() -> bool,
    ) -> Result<(), String> {
        if cancelled() {
            return Err("Undo cancelled before mutation".into());
        }
        let _settings = crate::workspace::settings::SETTINGS_MUTATION_LOCK
            .lock()
            .map_err(|_| "Settings unavailable")?;
        let _proposals = self
            .proposals
            .lock()
            .map_err(|_| "Edit state unavailable")?;
        let mut record = recovery::load(config, change_id)?;
        let authorization = authorize(config, &record.conversation_id, &record.path, generation)?;
        if authorization.task_root != record.task_root
            || authorization.task_revision != record.task_revision
            || authorization.grant_id != record.grant_id
        {
            return Err(
                "Task or grant changed; automatic Undo unavailable; recovery retained".into(),
            );
        }
        let mut target = target::LockedTarget::open(&authorization.path)?;
        if target.read()? != record.after {
            return Err("File changed since apply; Undo conflict; recovery retained".into());
        }
        if cancelled() {
            return Err("Undo cancelled before mutation".into());
        }
        record.state = "undoPrepared".into();
        recovery::append(config, &record)?;
        target.write(&record.before)?;
        record.state = "undone".into();
        recovery::append(config, &record)?;
        Ok(())
    }

    pub fn discard(&self, proposal_id: &str) -> Result<(), String> {
        self.proposals
            .lock()
            .map_err(|_| "Proposal state unavailable")?
            .remove(proposal_id);
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod recovery_tests;
#[cfg(test)]
mod tests;
