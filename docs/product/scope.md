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
