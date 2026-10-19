# Desktop foundation

## Approved stack

Tauri 2, React, TypeScript and Rust. Prefer bounded, event-driven work; idle should
not trigger AI requests, indexing or animations. Measure actual resource use later;
framework choice alone is not a performance guarantee.

## First slice

REQ-F01: launch a standalone Windows desktop shell with a clear chat-first identity.
REQ-F02: expose planned workspace sections without pretending integrations work.
REQ-F03: no external AI requests, file indexing or mutations on launch.

T001: scaffold the official React/TypeScript Tauri template, retaining repository docs.
T002: replace the template demo with an accessible responsive Yuki foundation UI.
T003: validate section navigation with runnable tests; run frontend and native builds.

| Requirement | Tasks | Acceptance |
| --- | --- | --- |
| REQ-F01 | T001, T002 | Desktop package compiles; chat is initial view |
| REQ-F02 | T002, T003 | Navigation works; pending features are labeled |
| REQ-F03 | T002, T003 | No application network/filesystem operations on startup |

This slice intentionally excludes model connectivity, project access, configurable
persisted data root, editing, schedule execution and avatar generation. These are
subsequent slices of the accepted incremental approach, not implemented placeholders.

## Structure

React renders the interface. Rust/Tauri owns privileged native operations when added.
Do not expose a generic shell or unrestricted filesystem capability. See
[boundaries](boundaries.md) for memory, tools and future collaboration constraints.
