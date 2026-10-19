# Current handoff

## State

Tauri 2 + React/TypeScript + Rust approved and scaffolded. Chat-first navigation
works; integrations are explicitly marked unavailable. Read [product scope](product/scope.md),
[boundaries](architecture/boundaries.md) and [foundation acceptance](architecture/foundation.md).
No model, filesystem, memory, scheduling or editor integration is implemented yet.

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
Completed prototype: user-approved Dream Arcade chat-home/navigation redesign; see
[visual direction](product/visual-direction.md). Parent implemented theme behavior/tests;
Coding subagent completed App.css after a failed initial model configuration. Auto
follows workspace; manual modes stay locked for this session, without persistence.
Sound and model integrations remain absent. Combined validation: 3 UI tests passed,
TypeScript/Vite build passed, native Windows release build passed. No browser/native
visual inspection or measured idle resource usage yet. Await user visual review before
spreading this design to other working surfaces. Then resume configurable data root/project access
and verified 9router connectivity. Future scope is not blanket approval for
self-modification, remote sharing or system-wide access.
