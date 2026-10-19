# Current handoff

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
Manual visual review remains pending. Notes scope question was interrupted and is not
approved for implementation. Do not delete real history during tests. Future scope
and verified 9router connectivity. Future scope is not blanket approval for
self-modification, remote sharing or system-wide access.
