# Local workspace storage

## Implemented slice

Settings stores a data-root pointer, approved project roots, and default-off
provider-sharing grants under the Windows app-config directory. Native folder
selection creates a dedicated Yuki data folder and checks writability. D: is a
preference, not an assumed available drive. Changing an existing nonempty or
unavailable data root is rejected; safe migration is not implemented.
Cancellation retains settings. No original project files are deleted.

Projects supports explicit folder approval, revocation, provider-sharing toggle,
and on-demand immediate child listing. Native commands recheck approved canonical
roots, traversal, and system/secret restrictions. Unsafe links are rejected;
listings are capped at 500 entries. Files are listed and read via bounded chunk
contracts only: content editing, file modification, write actions, and remote
uploads are absent in this slice. Browser preview cannot invoke native folder
or file operations.

## Approved read-only access and provider sharing

Native foundation provides approved read-only local file access and per-project
provider-sharing authorization without wiring file contents to remote AI providers:

1. **Workspace settings and provider-sharing grants**:
   - `WorkspaceSettings` includes `providerSharingGrants: Vec<String>`, defaulting
     to empty (`default-off`) for backward compatibility with legacy settings files.
   - Provider sharing can only be enabled for canonical approved project roots via
     `toggle_project_provider_sharing(root, enabled)` or `set_project_provider_sharing(root, enabled)`.
     Enabling sharing for unapproved paths is rejected.
   - When project root approval is revoked via `remove_project_root`, any active
     provider-sharing grant for that root is automatically cleared.

2. **Chunked local file reading (`read_local_file_chunk`)**:
   - Contract: accepts absolute ordinary local file path (`path`), byte offset (`offset`),
     and optional bounded chunk size (`chunk_size`, clamped to max 64 KiB, default 64 KiB).
   - Returns `FileChunk { path, text, offset, nextOffset, totalBytes, eof }`.
   - No arbitrary total file-size cap: large files can be read sequentially via chunk offsets.
   - Handles UTF-8 boundaries cleanly: chunks ending mid-character roll back to the
     nearest complete code-point boundary; `nextOffset` points to the start of the rolled-back
     character. Single-character expansion guarantees forward progress even on small chunk requests.
   - Rejects binary content (null bytes), invalid UTF-8 sequences, truncated sequences at EOF,
     non-regular files (directories, devices, pipes), and out-of-bounds offsets.
   - Privacy-preserving: error messages are strictly generic and content-free; file contents
     are never logged, traced, or persisted.
   - Broad ordinary local file reads do not require project-root approval; read permission
     does not grant remote provider transmission or command execution.

3. **Provider-sharing authorization (`authorize_provider_file`)**:
   - Reusable native validation function for future AI tool loops.
   - Rechecks all local read safety restrictions, plus enforces that the file is strictly
     contained within an approved project root with an active provider-sharing grant.
   - Verifies that intermediate symlinks or junctions do not escape the containing project root.
   - Does NOT transmit file contents to 9router or any provider in this foundation slice.

4. **Centralized security policy and Windows path safety**:
   - Rejects DOS device names anywhere in path components (`CON`, `PRN`, `AUX`, `NUL`,
     `COM1`..`COM9`, `LPT1`..`LPT9`, including stems such as `nul.txt`).
   - Rejects UNC paths (`\\server\share`, `//server/share`) and device paths (`\\.\`, `\\?\`).
   - Hardens common credential and cloud store directories: `.ssh`, `.aws`, `.azure`,
     `.kube`, `.gnupg`, `.docker`, `.gcloud`, `.password-store`, `secrets`.
   - Excludes secret and credential files: `.env`, `.env.*`, `credentials`, `credentials.*`,
     `.npmrc`, `.pypirc`, `.netrc`, `_netrc`, `secrets.*`, private keys (`.pem`, `.key`, `id_*`).
   - Protects system and OS locations: `windir`, `SystemRoot`, `ProgramFiles`, `ProgramData`,
     `AppData`, recycle bins, and drive/user profile roots.
   - Protects Yuki application settings directory (`config_dir`) and active data root (`data_root`).

5. **Bounded search operations (`search_project_files` and `search_project_text`)**:
   - Operate exclusively on share-enabled approved project roots.
   - Enforce centralized security policy: rechecks root approval, sharing grant, path containment,
     and skips symlinks/junctions, build/cache folders (`Library`, `Temp`, `node_modules`, `target`, `bin`, `.git`, etc.),
     system folders, and credential/secret files on every directory and file inspected.
   - Non-regex search: filename search uses an iterative wildcard matcher (`*`, `?`) or literal substring;
     text search uses literal string matching with Unicode-aware case-sensitivity toggles. Arbitrary regex evaluation is forbidden.
   - Streaming text search: reads files using 32 KiB chunk streaming buffers without arbitrary whole-file size rejection.
     Carries over boundary-spanning match windows across chunk boundaries without duplicate emissions. Automatically rolls
     back incomplete multi-byte code point splits at chunk ends.
   - Bounded context snippets: returns line number, byte offset, and a short snippet (clamped to 200 characters on UTF-8
     character boundaries around match with `...`), not entire file contents.
   - Budgeting and truncation: caps directories scanned (1000), files inspected (200), bytes inspected (20 MiB),
     execution time (5s), and serialized result payload size (60 KiB). Reports explicit coverage metadata (`complete` | `partial`),
     `limitsReached`, `unscannedRemainder`, `unsupportedBinaryFiles`, and cursor pagination tokens.
   - Pre-send reauthorization: before any result is transmitted to a provider, active project root sharing grants are rechecked;
     revoked root entries are purged and flagged with `revoked_grant`.

## Frontend integration

Projects connects file entries and an explicit absolute local file path to a
local-only plain-text preview. Explicit paths may be outside approved project roots;
native policy remains authoritative. Browser preview disables local controls and
never invokes filesystem commands. No folder search, indexing, editing, command
execution, AI context attachment or upload is introduced.

- Reads invoke `read_local_file_chunk` with `{ path, offset, chunkSize: 65536 }`.
  Tauri maps `chunkSize` to Rust's `chunk_size`. The UI uses the returned canonical
  `path`, `offset`, `nextOffset`, `totalBytes` and `eof`; ranges are labeled as
  half-open byte intervals `[offset, nextOffset)`, not character counts.
- Next range uses the exact returned `nextOffset` (including UTF-8 boundary
  adjustments); Back to start uses zero. Each read clears and replaces the current
  chunk rather than accumulating text or offset history. There is no total-size cap.
  Content is rendered as escaped text, not HTML or Markdown.
- New file reads, path edits, folder/project navigation, revocation attempts,
  section changes and closing clear displayed content. Read generation IDs reject
  stale successes/failures; unmount invalidates pending reads. Failed reads leave
  no old text visible. No preview content is persisted, logged or sent to providers.
- Per-root sharing reads `providerSharingGrants` (missing means off). Enabling
  opens an inline confirmation naming the approved root, 9router and upstream
  model providers; cancel performs no command. Confirmation calls
  `set_project_provider_sharing` with `{ root, enabled: true }`; disabling calls
  the same command with `enabled: false` without confirmation. Only a successful
  returned settings value changes the displayed grant; failures retain old state.
  This stores authorization only: actual AI file sharing is not implemented.

Focused DOM tests cover chunk replacement/navigation, explorer paths, failure
clearing, stale requests, close/section/unmount invalidation, revocation failures,
sharing consent/cancel/persistence failures, and browser isolation. These checks do
not establish real native picker/file IPC or rendered desktop/mobile acceptance.

## Limitations

- Path security is pattern- and location-based; it does NOT perform exhaustive content-based
  secret detection, data loss prevention (DLP), or entropy scanning on file bodies.
- Only UTF-8 text encodings are supported; binary files, UTF-16/32, and legacy non-UTF-8
  code pages are intentionally rejected.
- Native permission/chunk-reading and frontend local preview/sharing controls are
  integrated. AI tool loops, model prompt wiring, automated tool calling and actual
  provider file transmission remain unimplemented.
- Concurrent external writers to `settings.json` remain unsupported; atomic replacement uses
  backup/rollback rename sequences.

## Validation state

64 native tests cover settings persistence, atomic writes, legacy JSON backward
compatibility, directory listing, entry caps, symlink/junction containment, chunk range reads,
large file streaming, UTF-8 boundary management, binary/invalid text rejection, sensitive
file/directory blocking, Windows system directory protection, provider-sharing toggle/revocation,
and argument validation. All tests compile and pass cleanly without warnings.

Integration clarification: text-search nonzero cursors are currently rejected rather
than claiming reliable continuation. Narrow the root or file_pattern after partial
coverage. Filename search retains its match-offset cursor. Actual text inspection is
bounded to 4 MiB per operation, 200 files and 5 seconds, with per-directory enumeration
capped at 20,000 entries. An unfinished file scan is explicitly partial; these are
operation budgets, not a total on-disk file-size prohibition.
