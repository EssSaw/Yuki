# 9router chat slice

## Approved scope

User-configured 9router endpoint, explicit connection/model discovery, streamed
text chat with cancellation/errors, and original conversation history under the
selected data root. No automatic provider requests, project context, attachments,
retrieval, coding tools or Unity MCP in this slice.

## Requirements and acceptance

- C01: save endpoint/model locally; keep API credentials in Windows credential storage,
  never plaintext settings/history, logs or returned API payloads.
- C02: discover models only on explicit test action; no implicit provider fallback.
- C03: subscribe before generation, correlate stream events by request ID, prevent
  duplicate active requests and preserve cancellation/partial-response outcomes.
- C04: retain original user/assistant text with outcome status and reopen conversations.
  Bound prompt context without destructively compressing stored history.
- C05: require a valid configured data root before chat; report failures without
  inventing a response. Browser preview has no native/provider access.
- C06: automated checks use isolated temporary storage and local fixtures, not real
  personal data, credentials or external inference endpoints.

## Ownership and trust

Two Coding subagents own native services and frontend respectively; parent coordinates
contracts, integration, verification and documentation. Native code owns network access,
credentials, history paths and execution state. UI uses a narrow command/event API.
Treat generated text as plain text, not trusted HTML or executable instructions.

Configured remote endpoints receive explicitly submitted messages and bounded recent
conversation context. A loopback router may forward to remote upstream providers;
users must choose authorized routes and retention policies. Never claim local inference
merely because the base URL is localhost. No actual endpoint is tested by the agent.

## Validation state

Integrated: 43 native tests and 24 frontend tests passed, along with TypeScript/Vite
build and diff-format checks. Native Windows release compilation passed. No real provider
request or credential-manager round trip has been performed. GUI interaction and
actual route/model support require manual checks.

## Stream completion and layout follow-up

Completion accepts an explicit OpenAI terminal finish_reason as well as [DONE].
Fixtures reproduced false Failed outcomes when a provider sent finish_reason without
[DONE]; unconfirmed EOF still fails. Existing saved Failed messages are not rewritten.
The user's exact provider stream was not captured. Chat layout now uses a constrained
scrolling transcript, wider assistant reading measure and compact composer/mood controls;
manual desktop visual acceptance is still required.

## Native read-only tool-calling loop

The native chat streaming pipeline (`execute_chat_stream`) integrates an OpenAI-compatible
read-only tool-calling loop governed by strict containment, budgeting, and provider-sharing rules:

- **Available tools**:
  - `list_shareable_projects`: returns approved project roots that currently have explicit
    provider-sharing grants enabled.
  - `list_project_directory`: returns files and subdirectories within a granted root.
    Rejects unapproved roots, missing grants, traversal attempts (`..`), and restricted
    files/folders (.env, credentials, SSH keys, dot-directories, system folders).
  - `read_file_chunk`: reads bounded UTF-8 text (up to 64 KiB) from an authorized file
    inside a granted project root. Clamped to UTF-8 scalar boundaries.
- **Security & transmission boundary**:
  - No writes, shell execution, search/indexing, ungranted uploads, or background inference.
  - Reauthorization before send: immediately before any tool result payload crosses the
    network boundary to the provider, `reauthorize_tool_result` rechecks active root grants.
    If a user revoked sharing while execution was in flight, the pending content is purged
    and replaced with an authorization error.
  - Untrusted data framing: tool outputs and file text are sent with `role: "tool"` and
    treated strictly as data, never as prompt instruction overrides.
- **Budgeting & bounds**:
  - Maximum 5 tool rounds per user turn (`MAX_TOOL_ROUNDS`).
  - Maximum 8 tool calls per round (`MAX_TOOLS_PER_ROUND`).
  - Maximum 16 cumulative tool calls per user turn (`MAX_TOTAL_TOOL_CALLS`).
  - Maximum 64 KiB per tool result (`MAX_TOOL_RESULT_BYTES`).
  - Maximum 256 KiB cumulative tool output per user turn (`MAX_CUMULATIVE_TOOL_OUTPUT_BYTES`).
  - Maximum 64 KiB tool call arguments delta (`MAX_TOOL_ARGUMENTS_BYTES`).
- **History nonpersistence**:
  - Stored conversation history on disk retains only user and terminal assistant text messages.
    Transient tool calls, arguments, and file chunks are never persisted to conversation files,
    avoiding an accidental durable cache of sensitive project contents.
- **Plain chat compatibility**:
  - Standard chat models without tool-call deltas or completing with standard finish reasons
    (`stop`, `length`, `[DONE]`) stream and terminate normally without regression.

## Current limitations

History uses full JSON records with backup/rollback replacement, not transactional
multi-process storage. Prompt context is character-bounded, not an exact token count.
Failed or cancelled partial responses retain their statuses. Retrying explicitly sends
a new turn; it is not an idempotent replay. Settings/history writes have not undergone
forced-crash acceptance testing. Cancellation requests wait for terminal stream events;
quitting the process abruptly may lose an in-flight partial response. Tool calling is
restricted to native read-only inspection of granted projects; frontend visualization
of tool calls and status badges is pending future UI work (Hard UI required).
There is no long-term memory extraction, write capability, or shell execution.

## Manual setup

In desktop Settings, choose a data folder, save the 9router base URL and optional API
key, click Save connection & load models (saves the entered values before testing;
a failed test does not roll them back), select a model from the unfiltered Discovered
models dropdown or enter a custom ID, save settings, then send a non-sensitive
test message. Chat provides a compact model pill in the composer: open it to search
models or explicitly load/refresh the list. Selection saves the model for subsequent
turns without replacing the API key; switching and sends are blocked during that save.
The Conversations drawer supports reopening and confirmed permanent deletion. Native
deletion rejects active generation and removes only the selected history and matching
recovery artifacts; project files and credentials are untouched. Deletion failures keep
the conversation visible. Verified: 52 native tests, 26 frontend tests, frontend and
Windows release builds. Desktop visual/picker acceptance remains pending.
Test stop-generation and reopen the conversation after restart. Do not
paste credentials into chat. A locally routed endpoint may still use remote inference.

AI file chunks are capped at 8 KiB per tool read (local preview supports 64 KiB),
leaving space for JSON escaping under the serialized output budget. This is not a
file-size limit. Retained tool results are reauthorized on each subsequent request;
request-send waits also participate in cancellation. File contents in assistant-generated
answers can still appear in ordinary conversation history; excluding raw tool messages
from history does not imply that generated answers contain no project information.
