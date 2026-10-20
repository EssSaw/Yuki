# Native Notes Storage Architecture

## 1. Scope and Boundary Definition

The Native Notes module provides local-first, durable persistence for Markdown notes in Yuki under the user's selected and configured `data_root`.

### In-Scope Capabilities
- **Create note**: Generates a stable UUID v4 identifier, initializes a versioned record with display title and Markdown body, and writes it to disk.
- **List notes**: Enumerates note summaries (`id`, `title`, `createdAt`, `updatedAt`, `versionToken`) under `{data_root}/notes`, sorted by `updatedAt` descending.
- **Load note**: Retrieves the complete record (`id`, `title`, `body`, `createdAt`, `updatedAt`, `versionToken`) by UUID.
- **Save note**: Updates display title and/or Markdown body for an existing note using optimistic concurrency control. Note renaming does NOT change file naming or paths.

### Explicit Exclusions (Out of Scope)
- **No deletion**: File removal is not exposed in this foundation slice.
- **No AI note editing or automatic sharing**: Notes are not wired to model prompts or provider sharing in this slice.
- **No canvas integration**: Infinite canvas and graph nodes remain distinct future features.
- **No cloud synchronization / plugins**: Storage is strictly local-first and single-device.

---

## 2. Research Conclusions and Storage Design

### Single Versioned Record (`{uuid}.json`)
Research into local-first Markdown editors (Obsidian, Logseq, VS Code) highlights a frequent source of corruption: **split-file atomicity failure**. When metadata, title, and body are separated (e.g. frontmatter in one file, index in SQLite, and body in a raw `.md` file), partial writes or crashes can cause the index and content to desynchronize permanently.

Yuki resolves this by storing each note as a single, self-contained JSON record:
```json
{
  "schemaVersion": 1,
  "id": "e3b0c442-98fc-1c14-9af4-6996a707e429",
  "title": "Meeting Summary",
  "body": "# Notes\n\n- Local storage established\n",
  "createdAt": "2026-10-20T14:30:00.000Z",
  "updatedAt": "2026-10-20T14:35:00.000Z"
}
```
Storing title and body in one versioned document guarantees that title updates and body edits succeed or fail together.

### Stable UUID Identity
Notes are identified exclusively by canonical lowercase UUID v4 strings.
- Filename on disk is strictly `{uuid}.json`.
- The user-facing display title is an internal property of the record, not the filesystem name.
- Renaming a note's title **never** triggers a filesystem rename or path mutation.
- This eliminates filename collisions, OS path length limits (Windows `MAX_PATH`), illegal character restrictions (`:`, `*`, `?`, `"`, `<`, `>`, `|`), and case-sensitivity race conditions.

### Optimistic Concurrency and Byte Fingerprinting
To prevent silent overwrites when a user has multiple windows or when files are edited externally:
1. When loading or saving a note, a `version_token` is computed directly from the raw on-disk bytes using 64-bit FNV-1a paired with exact byte length: `{hash:016x}:{len:x}`.
2. Saving requires the caller to provide `expected_version`.
3. If an external editor (VS Code, Notepad, Git) or another concurrent operation modifies any byte on disk, the token changes and `save_note` fails immediately with `VersionConflict`.
4. **No forced overwrite**: Yuki does not permit bypassing version verification.

### Internal Concurrency Serialization
All state-modifying operations (create, save, conservative recovery) acquire an in-process mutex (`WRITE_MUTEX`). This prevents concurrent threads within Yuki from racing on temporary or backup filenames.

---

## 3. Path Security and Boundary Enforcement

1. **Data Root Validation**:
   - Rejects device paths (e.g. `\\.\`, `\\?\`) and UNC network shares (`\\server\share`).
   - Rejects DOS reserved device names anywhere in path components (`CON`, `PRN`, `AUX`, `NUL`, `COM1..9`, `LPT1..9`).
   - Validates that the configured data root exists and is a directory.
2. **Symlink and Reparse Point Rejection**:
   - Walks all parent ancestors of the data root up to the drive/filesystem root.
   - Rejects if any ancestor or the data root itself is a symbolic link or Windows NTFS reparse point / junction point.
   - Enforces the same check on `{data_root}/notes` and individual note files.
3. **No User-Supplied Paths**:
   - Note operations accept only a validated UUID string.
   - `validate_note_id` strictly rejects path traversal characters (`/`, `\`, `..`), non-hex characters, and non-v4 UUIDs.
4. **Finite Input Limits (Enforced Pre-Write)**:
   - **Title limit**: Maximum 200 Unicode characters (`MAX_NOTE_TITLE_CHARS`).
   - **Body payload limit**: Maximum 4 MiB (`MAX_NOTE_BODY_BYTES`).
   - **Serialized record limit**: Maximum 4 MiB + 64 KiB envelope (`MAX_NOTE_RECORD_BYTES`).
   - Violations fail in memory before creating temporary files or touching durable storage. Failed saves preserve the existing note file.

---

## 4. Atomic Replacement and Power-Loss Reality

### Atomic Save Protocol
1. Open unique temporary file `.{uuid}.json.tmp_{pid}_{nanos}` with `create_new(true)`.
2. Write serialized JSON payload and invoke `file.sync_all()` to flush buffers to storage media.
3. Move existing `{uuid}.json` to backup file `.{uuid}.json.bak_{pid}_{nanos}` via `std::fs::rename`.
4. Move temporary file to `{uuid}.json` via `std::fs::rename`.
5. If step 4 fails, roll back original file from backup to `{uuid}.json`.
6. Upon successful replacement, remove the backup file.

### Honest Assessment of Power-Loss Limitations
Standard OS file operations (`std::fs::rename`, `sync_all`) are **not** fully power-loss transactional on consumer filesystems (NTFS, ext4, APFS):
- While `file.sync_all()` forces file content to media, directory entry modifications (renames) rely on filesystem metadata journaling. On sudden power loss or kernel panic between renames, directory entries can be lost or restored to an intermediate state.
- On Windows, third-party software (antivirus filters, backup tools, Windows Search indexers) can transiently hold opportunistic read locks on `{uuid}.json`, causing `rename` to fail.
- Yuki handles this gracefully:
  - If the replacement fails, the rollback re-establishes the original note.
  - If rollback also fails, the backup file is preserved on disk, and `AmbiguousRecovery` is reported rather than deleting files.

### Conservative Recovery Principles
- **Corrupt Data Protection**: If an existing `{uuid}.json` contains invalid JSON syntax or truncated bytes, `save_note` and `load_note` reject the operation with `CorruptData` and **never overwrite** the damaged file.
- **Future Schema Protection**: If `{uuid}.json` contains `schemaVersion > CURRENT_NOTE_SCHEMA_VERSION`, `save_note` and `load_note` reject the operation with `FutureSchemaVersion` and refuse to overwrite or downgrade the file.
- **Orphan Backup Recovery**: If `{uuid}.json` is missing and exactly one valid backup file exists, `load_note` and `list_notes` safely restore it.
- **Ambiguous Recovery**: If multiple backup files exist, recovery preserves all files on disk and returns `AmbiguousRecovery` for manual resolution.

---

## 5. Exported Contracts and Types

### Types (`src-tauri/src/notes/models.rs`)
```rust
pub struct NoteRecord {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub version_token: String,
}

pub struct NoteSummary {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub version_token: String,
}
```

### Tauri Commands (`src-tauri/src/notes/commands.rs`)
- `create_note(app: AppHandle, title: String, body: String) -> Result<NoteRecord, String>`
- `list_notes(app: AppHandle) -> Result<Vec<NoteSummary>, String>`
- `load_note(app: AppHandle, id: String) -> Result<NoteRecord, String>`
- `save_note(app: AppHandle, id: String, title: String, body: String, expected_version: String) -> Result<NoteRecord, String>`

---

## 6. Module Registration Snippet for Parent Integration

When the parent agent integrates the notes module into `src-tauri/src/lib.rs`:

```rust
// In src-tauri/src/lib.rs:
pub mod chat;
pub mod mcp;
pub mod notes; // <-- Add this module declaration
pub mod workspace;

// In tauri::generate_handler![...]:
    notes::commands::create_note,
    notes::commands::list_notes,
    notes::commands::load_note,
    notes::commands::save_note,
```
