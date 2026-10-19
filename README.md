# Yuki

A personal, open-source AI companion for Windows: chat, project-aware coding help,
notes and canvas, and a local schedule in one standalone desktop app.

## Status

Planning stage. There is no runnable application yet. Desktop technology and the
first implementation slice are not yet approved. AI integration targets 9router;
local storage does not imply local inference.

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
using the configured Git identity. Build/test instructions will be added when
the application stack is selected; no tool installation is required for these docs.

Contributors and coding agents should start with [AGENTS.md](AGENTS.md).

## License

[MIT](LICENSE).
