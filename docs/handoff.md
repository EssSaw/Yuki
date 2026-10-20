# Current handoff

## Official Unity relay integration (2026-10-20)

User initial HTTP connection failed with SESSION_404; supplied relay metadata identifies
Unity Technologies official stdio relay, not CoplayDev HTTP. Research verified external
client launch relay_win.exe --mcp and Unity Pending Connections approval. User explicitly
approved stdio adapter but FIXTURES ONLY; real relay was not launched or modified.
Implemented separate executable grants, fixed-argument explicit launch, bounded newline
JSON-RPC init/inventory, owned-child cleanup/revocation and separate Settings controls.
Official editor binding remains pending/undocumented; no model-facing Unity tools or
provider transmission added. Initial HTTP404 now distinguished from expired sessions.
Validated 160 native tests, 95 frontend tests, frontend and Windows release builds.
Parent set Cargo default-run=yuki after fixture binary exposed Tauri ambiguity.
Next manual acceptance: approve official relay executable in Settings, explicitly connect,
accept Yuki in Unity Pending Connections, inspect capabilities then disconnect/revoke.
No live connection or rendered narrow visual acceptance claimed.


## Notebook redesign and local MCP sessions (2026-10-20)

User reports Notes functionality works, rejected its initial appearance, and requested
notebook styling. Hard UI implemented ruled ivory paper, binding/page cues, numbered
index and readable editor without changing save protections. Visual acceptance pending.
MCP native session layer and Settings controls now support explicit approved-endpoint
Connect, exact editor selection/binding, canonical approved-project verification, local
state refresh and Disconnect. No model-facing Unity tools or provider sharing through
MCP are implemented. Live Unity remains untested. Failed first native worker was resumed
from partial files; UI retry completed with Hard UI explicitly selected.
Validation: native suite passed (147 tests reported by native worker and independently
rerun), all 88 frontend tests passed, frontend and Windows native release builds passed,
diff formatting passed. No live MCP/private-data tests or rendered narrow-width review.
Next acceptance: approve/connect local running CoplayDev endpoint, select exact editor,
verify project/state, disconnect/revoke; then define narrow model-facing editor-state read.
Unrelated target/ preserved. Publish focused commits without attribution trailers.


## Notes and MCP discovery integrated (2026-10-20)

First local Notes editor/storage and MCP candidate discovery/approval Settings are
integrated. Notes: UUID records with Markdown body, explicit Save/Ctrl+S, title rename,
optimistic conflict detection, recovery candidates, Save/Discard/Cancel navigation and
native-close interception. No canvas/sync/delete/AI editing/sharing. Notes data is internal
versioned JSON, not interoperable standalone .md files. See architecture/notes.md.
MCP: fixed approved-project configs plus documented loopback candidate, explicit bounded
HTTP probing, persisted endpoint approval/revocation. No MCP initialize, editor binding
or Unity tools connected yet. See architecture/unity-mcp.md and research/integrations.md.
Parent fixed canonical note UUID validation, removed unused test imports and updated
MCP command fixtures in app tests. All 129 native and 79 frontend tests passed, frontend
production and Windows release builds passed. No native/manual Notes-close, recovery,
MCP live-endpoint or rendered desktop/narrow acceptance performed. No installs or real
provider/private-content tests. Publish focused integration with no attribution trailers;
retain unrelated target/. Next: manual Notes acceptance and MCP session/identity layer.


## Parallel tracks approved (2026-10-20)

User accepts current search provisionally: finds results, but search strategy/result
quality remain known follow-up debt. Search published as 24bfafd. Active MCP discovery
foundation (Coding) and version-specific connection/allowlist research (Research) have
separate ownership. User also approved first local Notes slice in parallel: create/open/
rename/edit Markdown notes under selected data root, explicit Save/Ctrl+S, visible
Unsaved/Saved/Failed state, Save/Discard/Cancel navigation protection; failed saves retain
drafts. No canvas/sync/plugins/AI note editing or sharing. Notes research started before
native persistence implementation. UI uses Hard UI after contract stabilizes; avoid
shared lib.rs/App.tsx edits concurrently with MCP integration. Parent owns this handoff.


## Search integration checkpoint (2026-10-20)

Implemented search_project_files (glob/literal filename matching) and
search_project_text (literal content matching) inside share-enabled roots, with
pre-send reauthorization, exclusions and explicit partial coverage. Parent corrected
text traversal after budget exhaustion, added enumeration/file/time guards, and rejects
nonzero text cursors until reliable continuation exists. Filename cursor remains.
Text scans use a 4 MiB operation budget, not a whole-file-size rejection. Unfinished
file scans explicitly mark coverage partial. All 102 native tests pass; frontend
suite and native release build (--no-bundle) passed. No real provider/private-file
search acceptance claimed. Ready for focused publication. User's live Unity-discovery
acceptance remains recorded below. Untracked target/ is unrelated and preserved.


## Latest feedback and active slice (2026-10-20)

User confirms live Unity-project detection works. Do not infer Godot/Unreal or exhaustive
coverage acceptance. Discovery/activity published as 17f7a1e. User approved bounded filename
and literal text search next. Coding agent owns native search/tool integration and related
architecture docs; no UI changes, writes/commands or MCP are authorized in this slice.
Enforce active sharing grants, exclusions, pre-send reauthorization and truthful partial
coverage. Parent will independently validate, update this handoff and publish verified work.


## Discovery and visible activity integration (2026-10-20)

Implemented marker-based Unity/Godot/Unreal discovery within share-enabled roots,
explicit coverage/limit metadata, final tool-disabled synthesis on budget exhaustion,
and transient request-correlated activity in chat. Primary integration tightened
per-entry enumeration limits and Unity marker-parent link checks. Tool activity shows
running/succeeded/failed or honestly stopped on termination, capped to 16 entries and
not reconstructed from history. Validated 87 native tests, 59 frontend tests, frontend
production and Windows native release build (--no-bundle), plus diff checks. No real
provider test or rendered desktop/narrow visual acceptance for this slice. Next manual
test: repeat game-project discovery in a specifically share-enabled non-sensitive test
folder and verify activity plus partial-coverage wording. Unrelated target/ preserved.


## Frontend tool activity (2026-10-20)

Implemented the approved bounded display in `src/ChatPanel.tsx` and
`src/ChatPanel.css`, with focused `src/ChatPanel.activity.test.tsx` regressions.
Matches native `toolActivity` id/name/status with optional summary (verified in
`chat/models.rs`); renders only the native safe summary, never raw metadata.
Current-request events update by activity ID, retain at most 16 entries and clear
on send, conversation load/new and successful deletion. Unfinished operations become
"Stopped - completion not confirmed" on done/error/cancel terminal or start failure;
cancel acknowledgement alone is not terminal and rejected cancellation stays running.
Completed/failed evidence remains honest; stale requests and late running updates
cannot revive completed activities. Activity is transient, outside the message live
log, with visible status text, bounded scrolling and no added animation.
Validation: 27 tests passed across activity/model/delete/Typography/VisualCheckpoint
(5 suites); TypeScript/Vite production build and `git diff --check` passed.
No native files, architecture docs, shared CSS, dependencies, provider calls,
private data, commits or pushes changed. DOM/source tests are not desktop/narrow
visual acceptance; rendered/native review remains pending. Parent integrates native
changes independently; stop after this UI slice.

## Latest user feedback and next action (2026-10-20)

Native chat tools are committed/published as 4b1f0fa; final validation was 76 native
and 46 frontend tests plus frontend/native release builds. User reports sharing
permissions work. Their broad game-project listing request hit the five-round limit;
no exhaustive discovery or full manual permission audit is claimed.
Recorded the requested all-rounder roadmap in [product scope](product/scope.md#all-rounder-assistant-roadmap-2026-10-20).
These are future capabilities, not blanket implementation/action approval. Recommended
next slice: bounded project discovery/search, visible tool activity and graceful partial
results; confirm whether to start this slice before treating the future note as approval.
Older entries below describe historical intermediate validation and publication state.

## Native read-only tool-calling loop (2026-10-20)

Implemented the approved native-only read-only tool-calling loop integrated with
streaming chat, cancellation, and original conversation history in `src-tauri/src/chat/`
and `src-tauri/src/workspace/`; contracts and bounds documented in [chat architecture](architecture/chat.md).
Exposes three strictly bounded read-only tools: `list_shareable_projects` (approved roots
with active provider-sharing grants only), `list_project_directory` (sanitized directory
listing within granted roots, secrets excluded), and `read_file_chunk` (bounded UTF-8 text
chunk reads within granted roots). Enforces pre-send reauthorization: revocation of a
project sharing grant while a tool is in flight immediately purges pending content before
crossing the network boundary. Enforces hard budgets (5 rounds, 8 calls/round, 16 total calls,
64 KiB/result, 256 KiB cumulative, 64 KiB arguments). No writes, shell, search/index,
uploads outside grants, provider fallback or automatic inference. Disk conversation history
strictly retains only user and assistant text messages; transient tool outputs and raw file
chunks are never persisted. Compatibility with plain streaming models without tools is preserved.

Validation: 75 native tests passed in `cargo test --manifest-path src-tauri\Cargo.toml`
(including mock HTTP SSE fixtures covering split arguments, tool success, ungranted/sensitive/traversal
denials, pre-send revocation, malformed/unknown tools, round budgets, cancellation, follow-up
assistant stream, and plain-chat regression). `cargo fmt --check` and `cargo clippy` passed cleanly.
No frontend UI changes (Hard UI model required later), no commits/pushes, no real provider requests.
Next: UI indicator/badges for tool activity using Hard UI per AGENTS.md.

## Local preview / sharing UI integration (2026-10-20)

Completed the approved frontend-only slice in `src/WorkspacePanel.tsx`,
`src/WorkspacePanel.css` and `src/WorkspacePanel.test.tsx`; contracts and privacy
boundaries are in [workspace storage](architecture/workspace-storage.md#frontend-integration).
Explorer entries and explicit absolute paths now open local-only bounded UTF-8
previews with byte range/size, next-range and back-to-start replacement navigation.
Preview generations invalidate on navigation, revocation, failure, close, section
changes and unmount. Per-root default-off sharing uses explicit 9router/upstream
confirmation before enable, direct disable and failure-preserved settings. Actual
AI file transmission remains absent. Browser preview invokes no native operations.

Validation: `npm test -- src\WorkspacePanel.test.tsx src\App.test.tsx
src\Typography.test.js` passed 32 tests (12 workspace tests); `npm run build`
passed TypeScript/Vite (29 modules); `git diff --check` passed. No native build,
real file/picker IPC, screenshots or desktop/narrow visual acceptance performed.
Preserved pre-existing native foundation, documentation and unrelated `target/`.
No dependencies, personal reads, provider/network requests, commits or pushes.
Stop after this integration; future AI tools must reauthorize at transmission.
This entry supersedes older statements below that frontend preview/toggles are absent.

## Native read foundation follow-up

Implemented native-only bounded UTF-8 local reads and default-off per-project provider
sharing grants. No frontend reader, AI tool loop or file upload is wired yet. Local
reads reject OS/secret/app-data paths, UNC/device paths and symlink/reparse ancestors;
no arbitrary total-file-size cap, maximum read chunk 64 KiB (with UTF-8 boundary handling).
Provider authorization rechecks approved root containment and the current sharing grant;
revocation removes grants. Policy is location/name-based, not exhaustive content DLP.
Primary assistant tightened ancestor link checks after integration review. All 64 native
tests and cargo fmt --check passed. Changes remain uncommitted pending frontend/tool
integration. Next: focused file preview and sharing controls using Hard UI, then bounded
AI read tools with provider authorization at the transmission boundary.


## Current checkpoint (2026-10-19)

User accepted the overall UI, readability improvements and sidebar fit, and requested
moving on. UI slice is ready for a focused commit on dev: scenery, stepped highlights,
native title bar, larger typography, Settings/Projects integration and 260px expanded
sidebar (64px collapsed). Final frontend validation: 37 tests passed, production build
passed. Native release compiled earlier; latest readability/sidebar changes have not
been rebuilt into that executable. Native-control and narrow-layout acceptance are
not fully documented; user visual feedback is not an automated screenshot check.
UI subagents must use Hard UI per AGENTS.md.

Next functional scope is not implemented: autonomous read-only file access with no
arbitrary total file-size cap (bounded range reads/context), no writes or commands.
User requests broad non-OS local access and per-project provider-sharing approval.
Local storage does not make model requests local: distinguish local disk access from
9router/upstream transmission before finalizing the access contract. Existing secret
and system restrictions must not be relaxed implicitly. No file-preview or AI-access
code belongs in the UI commit. Historical entries below retain earlier checkpoint
limitations and superseded preferences; this section is the current summary.

## State

Tauri 2 + React/TypeScript + Rust approved and scaffolded. Chat-first navigation
works; integrations are explicitly marked unavailable. Read [product scope](product/scope.md),
[boundaries](architecture/boundaries.md) and [foundation acceptance](architecture/foundation.md).
Native data-root settings and approved read-only project directory listing are implemented;
see [workspace storage](architecture/workspace-storage.md). No model, file-content editing,
memory, scheduling or Unity integration is implemented yet.

## Repository setup

Dedicated local repository inside the existing Yuki directory; parent repository
history/work is untouched. Origin is EssSaw/Yuki. Preserve the existing MIT license
and remote main history. Initial docs go to master, then development on dev.
Do not change GitHub's default branch without an explicit request.

## Validation

Documentation requires relative-link and formatting checks plus staged-diff review.
No app build applies until a scaffold exists. Integration research is documentary,
not a successful 9router or Unity runtime connection.

## Latest state and next action

Documentation links/whitespace and staged diff checks passed. Initial documentation
commit: `ef599c5`; updated commit authorization: `71e21e2`. After the user signed in,
master publication succeeded and dev was created/pushed. Both remote refs were
verified at `71e21e2` before this handoff update. Current development branch: dev,
tracking origin/dev. Existing remote main and the original license remain untouched.
The user authorizes focused, verified agent commits/pushes using the configured identity.

Foundation validation: npm test (2 passed), npm run build, and native release build
with npm run tauri build -- --no-bundle all passed. No installer build, visual desktop
review or idle-resource measurement yet. Native executable: src-tauri/target/release/yuki.exe.
Current prototype: Watercolor Sketchbook supersedes Dream Arcade, which the user
rejected as neon/cyberpunk and too sharp. See [visual direction](product/visual-direction.md).
Coding subagent replaced App.css with light paper, powder-blue/dusty-rose washes,
organic panels and painted hover reveals. Theme behavior is unchanged: Auto follows
workspace, manual modes lock for the session without persistence. Sound/model
integrations remain absent. Revalidated 3 UI tests, TypeScript/Vite build and diff
formatting. Native release was last built for the prior design; use tauri dev to view
the current stylesheet. No browser/native visual inspection or resource measurement
yet. Follow-up hover correction adds single-run 180ms painted backing snap/slide
and nudge/settle on enabled controls, with keyboard focus parity and reduced-motion
overrides. Local reference clip was inspected without upload or imported game assets.
Revalidated 3 tests and frontend build; exact perceived motion still needs user review.
Data-location settings and selected project access are now integrated: 15 Rust tests,
6 frontend tests, frontend build and native release compilation passed. Picker/restart
manual review remains pending. Nonempty/unavailable old data folders block relocation.
User requested multiple coding subagents for future independent implementation tasks;
rule saved. Current approved slice: [9router chat](architecture/chat.md), including explicit
connection settings, model discovery, streaming/cancel and local original history.
Two Coding agents delivered native and frontend separately; parent reconciled contracts
and corrected cancellation acknowledgement, active-guard error cleanup and unconfirmed
stream-end behavior. Validation: 34 native tests, 21 frontend tests, TypeScript/Vite
build and native Windows release build passed. No real provider requests or private-data
transmission during development. Manual acceptance still needed: Windows credential
storage, actual 9router model/stream support, cancel/reopen and restart persistence.
See chat documentation for setup and current limitations. No automatic project context
or tool execution in this slice. Then resume configurable data root/project access

Current follow-up integrated: native fixtures reproduced ignored terminal finish_reason
without [DONE], producing false Failed outcomes. Completion now accepts either signal;
truncated EOF stays Failed. Exact live-provider cause remains unconfirmed; old history
statuses are not rewritten. Layout improves reading width, type/spacing, transcript
scrolling and compact composer/mood controls. Stale Design preview footer removed.
Separate Coding agents owned Rust and CSS; parent integrated and verified 43 native
and 24 frontend tests plus frontend build. Native Windows release rebuild passed.
No actual endpoint requests or personal history inspected. Await user desktop visual
review and a fresh non-sensitive chat test; don't claim screenshots/runtime acceptance. Then resume configurable data root/project access

Chat usability slice complete: parent directly redesigned the compact searchable model
popover/composer and conversation drawer, per user override after picker agent cancellation.
Native deletion agent supplied UUID-bound deletion and recovery-artifact cleanup with
active-request exclusion. Parent added confirmation and failure-preservation UI tests.
Verified: 52 native tests, 26 frontend tests, frontend and Windows release builds.
User requested a whole-interface visual rework after rejecting the accumulated design.
Parent replaced App/Chat/Settings/Workspace styles with a cohesive shared watercolor
system: open transcript, muted washes, static grain, compact controls and consistent
spacing. 26 frontend tests, frontend build and native Windows release rebuild pass. Browser
visual tooling is not installed, so no screenshot acceptance is claimed.
Visual checkpoint in progress: in-app header, sidebar, empty and populated chat slice
implemented in accordance with the latest user approvals. Scenery image copied to `public/landscape.png`
from user-supplied artifact, preserving the observatory at the right via `right center` background crop.
Reading surface refactored into a continuous mostly opaque cool ivory soft-edge surface; assistant text is
open without cards or glass; user messages styled in subtle muted blue. Compact humanist typography adopted
across UI and headings, discarding oversized serifs; small expressive Yuki wordmark applied. Navigation
and controls use clean vector SVG icons, with stepped few-silhouette hover/focus reveals on static click targets.
Sidebar defaults to collapsed 64px icon rail with remembered user choice in localStorage. Functional
9router connection settings, model picker, streaming chat, cancellation, and history deletion remain fully intact.
Validation: 52 native tests passed (`cargo test`), 32 frontend tests passed (`vitest run`, 26 existing + 6 new
regression tests in `src/VisualCheckpoint.test.tsx`), TypeScript and Vite production build passed cleanly (`npm run build`).
Package tools inspected; no browser screenshot runner is installed in devDependencies, so no automated visual
verification is claimed. Desktop visual review remains pending. No Git commit or push performed; initial `target/`
status preserved.

Primary assistant refinement: moved appearance controls into Settings, selected local
Candara/Calibri humanist typography, softened the reading surface, fixed chat width plus
margins overflowing narrow viewports, corrected the mobile grid, and guarded preference
reads when localStorage is blocked. Added storage-failure regression coverage.
Revalidated npm test (33 passed) and npm run build. Native custom titlebar and actual
desktop/narrow screenshot inspection remain pending; do not call this visually approved.
User requires the primary assistant to personally handle subsequent UI implementation.
No commit or push before visual approval.

Primary assistant implemented the missing continuous stepped highlight loop for navigation,
welcome shortcuts, New chat and enabled Send; labels and hit targets remain stationary.
Added pointer-driven scenery parallax, short workspace transitions, native drag region and
minimize/maximize/close controls with error feedback and narrowly scoped Tauri permissions.
Native chrome disabled; removed misleading connection-status dot. Reduced motion disables
animation and parallax. Verified 34 frontend tests, frontend production build, native
release build (--no-bundle), and diff whitespace checks. No screenshot/browser inspection
or native window interaction acceptance yet; these remain required before visual approval.
No commit or push. The earlier pending-titlebar implementation note is superseded by this entry.

Context refresh: reread repository/starter instructions and visual checkpoint decisions.
User gave positive visual feedback on the current look; preserve it rather than redesigning.
This does not establish narrow-width or native-control acceptance. Primary assistant
closed the remaining hidden-window motion gap: visibility changes pause decorative CSS
animations, with listener cleanup on unmount and regression coverage. Targeted checkpoint
suite: 9 tests passed; frontend build and diff checks passed. Native executable predates
this last visibility refinement. Continue with narrow-width/native interaction acceptance
before propagation or publication; no new feature scope has been approved.

Updated user preference: UI work must call a Hard UI subagent, superseding the earlier
primary-assistant-only restriction. The authoritative model-selection rule is in
[AGENTS.md](../AGENTS.md); primary assistant remains responsible for integration and
validation. No UI implementation was performed as part of this rule update.

Readability pass (2026-10-19): preserved the existing visual checkpoint and dirty work.
Updated `src/design-system.css`, `src/App.css` and `src/ChatPanel.css` in place:
shared body/chat/composer text is 16px (previous chat 14/14.5px), controls 14px,
secondary text 13px, captions 12px (previously 9-10.5px). Chat line-height is
1.7 assistant / 1.65 user; composer 1.6. Sidebar/topbar toggles are 36px,
send is 40px (was 32px), nav retains 44px targets, and titlebar is 48px.
Header actions wrap; composer model text can shrink; model popover is bounded
inside composer width with viewport-height scrolling; history rows wrap long
names and preserve delete targets. Narrow layouts retain the readable type scale.
Scenery, humanist family, stepped highlights, reduced motion and hidden-window
pause remain intact. Added `src/Typography.test.js` for source-level size and
popover constraints; this is not a browser layout test.
Final validation: `npm test -- src\\Typography.test.js src\\VisualCheckpoint.test.tsx
src\\App.test.tsx src\\ChatPanel.model.test.tsx src\\ChatPanel.delete.test.tsx`
passed 31 tests across 5 files; `npm run build` passed TypeScript and Vite (29
modules); `git diff --check` passed. An initial CSS raw-import assertion was
unsupported by the test transform; the final standalone JS test reads the CSS
without adding Node types or dependencies. No provider requests, personal data,
commit/push, or native build. Existing native rebuild was not touched. Browser
screenshot tooling remains unavailable; desktop/narrow visual acceptance and
native rendering of this new scale remain pending.

Settings / Projects continuation (2026-10-19): extended the accepted scenery and
cool-ivory direction without changing runtime behavior. Replaced the old serif,
undersized rules in `src/ConnectionSettings.css` and `src/WorkspacePanel.css`:
16px form/body text, 14px controls/paths, 12-13px supporting labels, 40px buttons
and 44px form fields. Settings and Projects now use continuous opaque ivory
surfaces with landscape margins, open sections and quiet row separators rather
than nested cards. Long paths/errors wrap; project actions stack at narrow widths;
connection actions wrap/stack and mood choices wrap without reducing type size.
Removed project-button lift so targets stay stationary. Existing global keyboard
focus, reduced motion and hidden-window pause remain authoritative.
Minimal shared changes in `src/App.css` give Settings its surface and align mood
controls; `src/App.tsx` moves the existing local Settings heading/data section
before appearance controls, leaving handlers, guards and component keys intact.
No connection/project/data-root logic, chat or shell redesign, dependencies,
provider requests or personal-data access. All pre-existing dirty work, including
`target/`, was preserved; no commit/push or native rebuild.
Validation: `npm test -- src\ConnectionSettings.test.tsx
src\ConnectionSettings.failure.test.tsx src\WorkspacePanel.test.tsx
src\App.test.tsx src\VisualCheckpoint.test.tsx` passed 32 tests in 5 files.
`npm run build` passed TypeScript and Vite (29 modules); `git diff --check`
passed; editor diagnostics reported no errors in the four edited source files.
`npm test -- src\Typography.test.js` also passed its readability regression.
Browser screenshots and desktop/narrow visual inspection were not performed;
CSS layout and native rendering still need user review. Stop after this slice.

Expanded-sidebar clipping follow-up (2026-10-19): inspected the supplied read-only
screenshot and current App source. `src/App.css` increases expanded width from
220px to 260px; brand subtitle, navigation labels and footer text can wrap rather
than clip/ellipsis. Font sizes are unchanged. The 64px collapsed rail, stored
preference logic and mobile horizontally scrolling navigation are unchanged;
mobile labels explicitly stay on one line. No `App.tsx` changes were needed.
Extended `src/Typography.test.js` with source-level width/wrapping/mobile guards.
Validation: 29 tests passed across Typography, VisualCheckpoint and App suites;
`npm run build` passed TypeScript/Vite (29 modules); `git diff --check` passed.
An initial newline-sensitive assertion failed on CRLF and was corrected before
rerunning. These are source/DOM checks, not rendered layout acceptance; no browser
or native visual validation/rebuild was performed. Preserved existing dirty work,
attachments and `target/`; no dependencies, commit or push. Bounded fix complete.

Parent integration review: native tool results retained in a turn are now reauthorized
before every subsequent provider request, not only immediately after execution. Request
send waits are cancellation-aware. Missing tool IDs are normalized once for matching
assistant/tool messages. AI reads default/clamp to 8 KiB so JSON-escaped content fits the
64 KiB serialized-result budget; local preview remains 64 KiB. Added escaped-content
budget regression; all 76 native tests pass. No live provider or manual acceptance yet.
Final integration validation: 76 native tests, 46 frontend tests, Rust formatting,
frontend production build and native Windows release build (--no-bundle) passed.
Executable: src-tauri/target/release/yuki.exe. Ready for focused commit/publication.
Actual configured-model tool support remains unverified. Manual acceptance must use
an explicitly share-enabled temporary project containing only dummy text; verify reading,
disabled-sharing denial, cancellation and plain chat. No private files/live provider
requests were used during automated validation.
