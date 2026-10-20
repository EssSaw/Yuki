# Product scope

## Confirmed direction

Yuki is the user's own extensible, open-source personal companion. Initial use is
single-user on Windows desktop, standalone and background-capable, not mobile.
It is a versatile chatbot, not a fixed productivity workflow on every launch.

The user develops games with Unity, primarily VS Code and sometimes Rider,
composes in FL Studio, and draws in Clip Studio Paint.

## Workspace capabilities

- Chat with project-aware coding assistance inside Yuki, not an IDE extension initially.
- File explorer and selected project roots for coding context.
- Editable text notes and a node/card canvas with behavior inspired by Obsidian;
  exact vault, plugin and canvas-file compatibility is not required.
- Local schedule/task manager: ask missing details, preview, then save on approval.
- AI access to the same capabilities: create or modify notes/canvas, propose schedules,
  manage tasks and assist coding through permission-controlled tools.
- Configurable data/memory location, preferring D: when available and chosen by the user.
- Personality and a static anime portrait initially; no unsolicited work interference.

Access may be granted to user-selected folders anywhere appropriate, but OS/critical
system locations and secrets are excluded. The accepted starting trust proposal is
allowlisted reads and previews/approval for writes, commands, deletion and invitations.
Authorization to use 9router is not blanket permission to upload all accessible files.

## Memory intent

Keep complete original conversation history without destructive compression. Use
recent context and relevant historical retrieval to support continuity while staying
focused on the active conversation unless the user redirects it. Exact extraction,
forgetting and private-chat behavior still need confirmation; perfect recall is not promised.

## All-rounder assistant roadmap (2026-10-20)

User requested recording these capabilities for future implementation, not blanket
permission to execute them or install integrations:

1. Read-only discovery/search: game-project detection using engine markers, filename
   and text search with exclusions, bounded traversal and pagination. Preserve folder
   sharing grants; report partial coverage rather than imply an exhaustive result.
2. Observable tools: visible running/succeeded/failed activity and useful partial-result
   handling when a tool budget is reached. Do not simply remove resource limits.
3. MCP connections: explicitly approved servers, narrow per-integration permissions,
   and the same native access/sharing enforcement. MCP is not a permission bypass.
4. Coding actions: previewed patches, builds/tests and Git operations, with separate
   write/command approval policies before implementation.
5. Personal workspace tools: notes/canvas, tasks/schedules and memory retrieval, each
   with confirmed storage, privacy and mutation rules.
6. Research and app integrations: cited web search/fetch, Unity, browser and selected
   creative-app tools. External sends, invitations, publishing and deployments require
   action previews and confirmation unless separately authorized.

Current implementation remains read-only file tools, local previews and explicit
provider-sharing grants. User reports sharing permission works in the desktop app;
this is user acceptance feedback, not exhaustive permission-boundary verification.
A live request to discover game projects hit the five-round tool budget. That is an
observed discovery/coverage limitation, not evidence that tools are absent.

## Approved staged Agent capabilities (2026-10-20)

User confirmed this permission design after grilling. It governs Yuki's runtime,
not this coding assistant's permission to operate personal projects during development.
First implementation slice is Agent mode, visible task scope and grant controls only;
file writes, commands and Unity mutations are not enabled by that slice.

- Dynamically discover/select projects; reuse a verified conversation target, ask on
  ambiguity and revalidate identity/permissions. Explorer right-click Ask Yuki to read
  is a planned context-selection interaction, not implicit upload permission.
- Ordinary Chat is non-mutating. Explicit Agent mode shows the active workspace/task
  and grants. Standing grants cover selected write folders, explicitly approved broader
  sharing roots and reviewed command templates/scripts. Files cannot self-authorize.
- Recoverable text edits use local before-images, change sets and external-conflict
  detection; never silently overwrite concurrent edits.
- Reviewed commands may be unsandboxed, disclosed explicitly: working directory is
  not containment. New/changed scripts, arguments and install hooks require review.
- Unity uses exact verified editor/project binding and task-level approval for reviewed
  operations, including object/component/asset changes, saving, play/tests/builds.
  Protect existing dirty scenes; verify Undo/snapshot coverage or ask before proceeding.
- Deletion/cleanup, Git commits/push/history changes, admin actions, external sends/
  deployments and out-of-scope writes require fresh approval. Arbitrary C# and
  unreviewed custom MCP tools require exact-payload approval even inside a task.
- Broad sharing retains secret/system/app-internal/link/cache exclusions and sends
  relevant content only. Name/path filters cannot guarantee content-secret detection.
- Stop blocks new actions and requests active cancellation; report unknown effects
  honestly. No automatic rollback, replay or restart-resume of uncertain mutations.

Delivery order: grants/mode, recoverable text edits, reviewed commands, verified Unity
reads/edits. Consistent UI reuses the accepted shared typography, palette, controls,
focus/motion conventions and status language; new surfaces must not introduce a new
visual system. Notes' notebook treatment remains an intentional specialized surface.

## Future direction, not initial commitments

- Collaborative synchronized notes/canvas with friends.
- Verified Unity MCP integration and broader coding-agent behavior.
- Launching specialist creative applications.
- Avatar expressions, animation and possibly voice.
- User-reviewed evolution of Yuki's appearance and interaction style. Autonomous
  self-modification, automatic code execution and permission expansion are not authorized.

## Decisions still open

Desktop stack and first coding slice; minimum coding-chat/editor behavior; reminder
exceptions outside chat; automatic memory categories; exact collaboration requirements;
9router installation/routes and upstream data retention. License is existing MIT.
