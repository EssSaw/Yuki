use serde::{Deserialize, Serialize};

/// Current schema version for note records on disk.
pub const CURRENT_NOTE_SCHEMA_VERSION: u32 = 1;

/// Maximum allowed length for a note display title in Unicode characters.
pub const MAX_NOTE_TITLE_CHARS: usize = 200;

/// Maximum allowed size for a note Markdown body in bytes (4 MiB).
pub const MAX_NOTE_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Maximum allowed serialized JSON record size in bytes (4 MiB body + 64 KiB envelope).
pub const MAX_NOTE_RECORD_BYTES: usize = 4 * 1024 * 1024 + 64 * 1024;

/// On-disk representation of a note record stored within `{uuid}.json`.
/// Single versioned JSON record containing display title, Markdown body,
/// and ISO-8601 timestamps. The version token is intentionally not stored
/// on disk, as it represents the exact content fingerprint of the serialized bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiskNoteRecord {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
}

impl DiskNoteRecord {
    /// Converts disk record into a public `NoteRecord` paired with an optimistic version token.
    pub fn into_record(self, version_token: String) -> NoteRecord {
        NoteRecord {
            schema_version: self.schema_version,
            id: self.id,
            title: self.title,
            body: self.body,
            created_at: self.created_at,
            updated_at: self.updated_at,
            version_token,
        }
    }

    /// Converts disk record into a lightweight `NoteSummary` for listing.
    pub fn to_summary(&self, version_token: String) -> NoteSummary {
        NoteSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            version_token,
        }
    }
}

/// In-memory and API representation of a full note record returned to frontend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NoteRecord {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub version_token: String,
}

/// Lightweight note summary returned when listing notes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NoteSummary {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub version_token: String,
}

/// Request payload to create a new note.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateNoteRequest {
    pub title: String,
    pub body: String,
}

/// Request payload to save/update an existing note.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SaveNoteRequest {
    pub id: String,
    pub title: String,
    pub body: String,
    pub expected_version: String,
}

/// Explicit error enum covering validation, storage, security, and concurrency failures.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum NoteError {
    DataRootNotConfigured(String),
    DataRootNotFound(String),
    DataRootInvalid(String),
    SecurityViolation(String),
    InvalidUuid(String),
    TitleTooLong { limit: usize, actual: usize },
    PayloadTooLarge { limit: usize, actual: usize },
    NoteNotFound(String),
    NoteAlreadyExists(String),
    VersionConflict { expected: String, actual: String },
    CorruptData(String),
    FutureSchemaVersion { expected: u32, actual: u32 },
    AmbiguousRecovery(String),
    IoError(String),
    SerializationError(String),
}

impl std::fmt::Display for NoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DataRootNotConfigured(msg) => write!(f, "Data root is not configured: {msg}"),
            Self::DataRootNotFound(msg) => write!(f, "Data root directory not found: {msg}"),
            Self::DataRootInvalid(msg) => write!(f, "Data root directory is invalid: {msg}"),
            Self::SecurityViolation(msg) => write!(f, "Security violation: {msg}"),
            Self::InvalidUuid(msg) => write!(f, "Invalid UUID: {msg}"),
            Self::TitleTooLong { limit, actual } => {
                write!(
                    f,
                    "Note title exceeds limit: {actual} characters (max {limit})"
                )
            }
            Self::PayloadTooLarge { limit, actual } => {
                write!(
                    f,
                    "Note payload exceeds limit: {actual} bytes (max {limit} bytes)"
                )
            }
            Self::NoteNotFound(msg) => write!(f, "Note not found: {msg}"),
            Self::NoteAlreadyExists(msg) => write!(f, "Note already exists: {msg}"),
            Self::VersionConflict { expected, actual } => {
                write!(
                    f,
                    "Conflict: note was modified externally or by another process (expected version '{expected}', found '{actual}')"
                )
            }
            Self::CorruptData(msg) => write!(f, "Corrupt note data detected: {msg}"),
            Self::FutureSchemaVersion { expected, actual } => {
                write!(
                    f,
                    "Note schema version {actual} is newer than supported version {expected}"
                )
            }
            Self::AmbiguousRecovery(msg) => write!(f, "Ambiguous recovery state: {msg}"),
            Self::IoError(msg) => write!(f, "I/O error: {msg}"),
            Self::SerializationError(msg) => write!(f, "Serialization error: {msg}"),
        }
    }
}

impl std::error::Error for NoteError {}

impl From<NoteError> for String {
    fn from(err: NoteError) -> Self {
        err.to_string()
    }
}
