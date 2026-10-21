use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_RECOVERY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RECORD_BYTES: u64 = 14 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRecord {
    pub schema_version: u32,
    pub id: String,
    pub conversation_id: String,
    pub path: String,
    pub task_root: String,
    pub task_revision: String,
    pub grant_id: String,
    pub state: String,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

fn directory(config: &Path) -> Result<PathBuf, String> {
    let settings = crate::workspace::settings::load_settings(config)?;
    let root = PathBuf::from(settings.data_root.ok_or("Choose a data root first")?);
    for ancestor in root.ancestors() {
        if crate::workspace::discovery::is_symlink_or_reparse(ancestor) {
            return Err("Unsafe recovery data path".into());
        }
    }
    let dir = root.join("edit-recovery");
    if !dir.exists() {
        std::fs::create_dir(&dir).map_err(|_| "Cannot create local recovery storage")?;
    }
    if crate::workspace::discovery::is_symlink_or_reparse(&dir) {
        return Err("Unsafe recovery storage".into());
    }
    Ok(dir)
}

pub fn append(config: &Path, record: &ChangeRecord) -> Result<(), String> {
    let dir = directory(config)?;
    let bytes = serde_json::to_vec(record).map_err(|_| "Cannot encode recovery record")?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("Recovery record exceeds supported size".into());
    }
    validate_record(record, &record.id, &record.state)?;
    let mut reservations = std::collections::BTreeMap::<String, u64>::new();
    for (index, entry) in std::fs::read_dir(&dir)
        .map_err(|_| "Cannot inspect recovery quota")?
        .enumerate()
    {
        if index >= 1024 {
            return Err("Recovery record quota reached; no files removed automatically".into());
        }
        let entry = entry.map_err(|_| "Cannot inspect recovery quota")?;
        if crate::workspace::discovery::is_symlink_or_reparse(&entry.path()) {
            return Err("Unsafe recovery entry".into());
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let parts: Vec<_> = name.split('.').collect();
        if parts.len() != 3 || parts[2] != "json" {
            return Err("Unknown recovery entry; review required".into());
        }
        let existing = read_record(&entry.path(), parts[0], parts[1])?;
        let size = serde_json::to_vec(&existing)
            .map_err(|_| "Cannot size recovery record")?
            .len() as u64
            + 64;
        let slot = reservations.entry(existing.id).or_default();
        *slot = (*slot).max(size);
    }
    let slot = reservations.entry(record.id.clone()).or_default();
    *slot = (*slot).max(bytes.len() as u64 + 64);
    // Reserve logical quota for prepared, committed, undoPrepared and undone.
    // This is not a promise that the OS cannot run out of disk space later.
    let reserved = reservations
        .values()
        .try_fold(0u64, |sum, size| sum.checked_add(size.saturating_mul(4)))
        .ok_or("Recovery quota overflow")?;
    if reserved > MAX_RECOVERY_BYTES || reservations.len() > 256 {
        return Err("Local recovery quota exhausted; no backups evicted".into());
    }
    let path = dir.join(format!("{}.{}.json", record.id, record.state));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "Cannot create recovery record; target not changed")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Cannot persist recovery record")
        .map_err(String::from)
}

pub fn load(config: &Path, id: &str) -> Result<ChangeRecord, String> {
    let uuid = uuid::Uuid::parse_str(id).map_err(|_| "Invalid change ID")?;
    if uuid.to_string() != id {
        return Err("Invalid canonical change ID".into());
    }
    let dir = directory(config)?;
    if dir.join(format!("{id}.undoPrepared.json")).exists() {
        return Err("Undo was already attempted; outcome requires local review".into());
    }
    let path = dir.join(format!("{id}.committed.json"));
    if crate::workspace::discovery::is_symlink_or_reparse(&path) {
        return Err("Unsafe recovery record".into());
    }
    let record = read_record(&path, id, "committed")?;
    let prepared = read_record(&dir.join(format!("{id}.prepared.json")), id, "prepared")?;
    if !same_change(&record, &prepared) {
        return Err("Recovery stages disagree; manual review required".into());
    }
    Ok(record)
}

fn same_change(a: &ChangeRecord, b: &ChangeRecord) -> bool {
    a.id == b.id
        && a.conversation_id == b.conversation_id
        && a.path == b.path
        && a.task_root == b.task_root
        && a.task_revision == b.task_revision
        && a.grant_id == b.grant_id
        && a.before == b.before
        && a.after == b.after
}

fn validate_record(record: &ChangeRecord, id: &str, state: &str) -> Result<(), String> {
    if record.schema_version != 1
        || record.id != id
        || record.state != state
        || uuid::Uuid::parse_str(id)
            .map(|value| value.to_string() != id)
            .unwrap_or(true)
        || !matches!(
            state,
            "prepared" | "committed" | "undoPrepared" | "undone" | "cancelled"
        )
    {
        return Err("Unsupported recovery record; preserved for review".into());
    }
    crate::chat::history::validate_conversation_id(&record.conversation_id)?;
    uuid::Uuid::parse_str(&record.grant_id).map_err(|_| "Invalid recovery grant ID")?;
    if !Path::new(&record.path).is_absolute() || !Path::new(&record.task_root).is_absolute() {
        return Err("Invalid recovery path; preserved for review".into());
    }
    super::validate_text(&record.before)?;
    super::validate_text(&record.after)?;
    Ok(())
}

fn read_record(path: &Path, id: &str, state: &str) -> Result<ChangeRecord, String> {
    if crate::workspace::discovery::is_symlink_or_reparse(path) {
        return Err("Unsafe recovery record".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "Recovery record unavailable")?
        .take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read recovery record")?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("Oversized recovery record".into());
    }
    let record: ChangeRecord = serde_json::from_slice(&bytes)
        .map_err(|_| "Corrupt recovery record; preserved for review")?;
    validate_record(&record, id, state)?;
    Ok(record)
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryStatus {
    pub id: String,
    pub state: String,
    pub review_required: bool,
}

pub fn inventory(config: &Path) -> Result<Vec<RecoveryStatus>, String> {
    let dir = directory(config)?;
    let mut states =
        std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    for (index, entry) in std::fs::read_dir(dir)
        .map_err(|_| "Cannot inspect recovery records")?
        .enumerate()
    {
        if index >= 1024 {
            return Err("Recovery inventory exceeds supported record count".into());
        }
        let entry = entry.map_err(|_| "Cannot inspect recovery entry")?;
        if crate::workspace::discovery::is_symlink_or_reparse(&entry.path()) {
            return Err("Unsafe recovery entry".into());
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let parts: Vec<_> = name.split('.').collect();
        if parts.len() != 3 || parts[2] != "json" || uuid::Uuid::parse_str(parts[0]).is_err() {
            return Err("Unknown recovery entry preserved; manual review required".into());
        }
        let record = read_record(&entry.path(), parts[0], parts[1])?;
        let prepared_path = entry
            .path()
            .with_file_name(format!("{}.prepared.json", record.id));
        if record.state != "prepared" {
            let prepared = read_record(&prepared_path, &record.id, "prepared")?;
            if !same_change(&record, &prepared) {
                return Err("Recovery stages disagree; manual review required".into());
            }
        }
        states
            .entry(parts[0].into())
            .or_default()
            .insert(parts[1].into());
    }
    Ok(states
        .into_iter()
        .map(|(id, states)| {
            let (state, review_required) = if states.contains("undone") {
                ("undone", false)
            } else if states.contains("undoPrepared") {
                ("uncertainUndo", true)
            } else if states.contains("committed") {
                ("committed", false)
            } else if states.contains("cancelled") {
                ("cancelled", false)
            } else {
                ("uncertainApply", true)
            };
            RecoveryStatus {
                id,
                state: state.into(),
                review_required,
            }
        })
        .collect())
}
