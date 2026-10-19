# System boundaries

These are design proposals and constraints, not implemented components.

## Shared operations

UI actions and AI tools should call the same validated application operations.
Keep permission checks, previews, cancellation, outcome reporting and audit records
outside model prompts. Separate Yuki's personality from code-work instructions;
file instructions apply to their project scope without becoming conversational identity.

## Local data and memory

Separate original history, recent-context selection, curated memories, source notes
and derived search indexes. Preserve original history; rebuild derived representations
without replacing originals. Bound retrieval/prompt size, and attach provenance to
memory. Never equate local disk capacity with model context capacity.

Choose a data root in settings; check availability, space and writability rather than
assuming D: exists. Relocation must be validated and recoverable before switching.
Secrets belong in platform credential storage, not notes/history/index metadata.
Deletion must cover applicable derived records; retention semantics remain to be designed.

## Tools and integration

Use selected project roots, reject escapes/system paths, and revalidate access at
execution. Read access, indexing, remote inference and mutation are separate rights.
Potential Unity MCP tools are privileged editor operations, not implicitly safe reads.
Keep the editor/project identity explicit and require approval for mutations.

## Scheduling lifecycle

Accepted baseline: local schedule, tray operation after window close, explicit Quit
stops Yuki, missed reminders shown on next launch. Do not promise delivery while the
PC is off. Confirm notification permissions and timezone behavior before implementation.

## Collaboration seam

Avoid baking a single-device identity into note/canvas content. Consider stable
record IDs, revisions and explicit operation boundaries. Do not choose a sync server,
CRDT framework or conflict policy before collaboration behavior is agreed.
