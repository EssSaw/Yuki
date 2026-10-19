# Yuki

A personal, open-source AI companion for Windows: chat, project-aware coding help,
notes and canvas, and a local schedule in one standalone desktop app.

## Status

Desktop foundation: Tauri 2 + React/TypeScript + Rust. Chat-first navigation is
implemented, with native data-folder settings and approved read-only project browsing.
9router connection settings, model discovery, streaming text chat and original local
conversation history are implemented. File-content preview, notes/canvas, schedule
and long-term memory retrieval are not yet implemented. AI integration targets 9router; local storage
does not imply local inference.

## Product direction

- Respond when called; no mandatory startup routine or unsolicited work monitoring.
- Let the user and AI operate the same notes, canvas, tasks, schedule and project files.
- Preview and approve consequential actions, including schedule commitments and code edits.
- Preserve original conversation history; use bounded recent context and searchable memory.
- Keep data location configurable, preferring an available user-selected D: drive location.
- Begin with a static anime portrait; expressions, voice and personalization may follow.

## Documentation

Start with [the documentation index](docs/README.md). Confirmed scope is separate
from proposals and unresolved decisions. Future features are not current capabilities.

## Development

`master` holds the initial documentation baseline. Application development happens
on `dev` after the initial publication, with focused, verified commits and pushes
using the configured Git identity.

Prerequisites: Node/npm, Rust, Windows C++ Build Tools and WebView2.

```powershell
npm ci
npm test
npm run build
npm run tauri dev
```

`npm run dev` previews the UI in a browser, not the native desktop integration.
`npm run tauri build -- --no-bundle` compiles the Windows executable without an
installer. Native build output is under `src-tauri/target/release/`.
No AI calls or file indexing run on startup. Local settings are written only through
explicit folder approval/configuration actions; project source files remain read-only.

Contributors and coding agents should start with [AGENTS.md](AGENTS.md).

## License

[MIT](LICENSE).
