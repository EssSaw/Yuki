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

Integrated: 34 native tests and 21 frontend tests passed, along with TypeScript/Vite
build and diff-format checks. Native Windows release compilation passed. No real provider
request or credential-manager round trip has been performed. GUI interaction and
actual route/model support require manual checks.

## Current limitations

History uses full JSON records with backup/rollback replacement, not transactional
multi-process storage. Prompt context is character-bounded, not an exact token count.
Failed or cancelled partial responses retain their statuses. Retrying explicitly sends
a new turn; it is not an idempotent replay. Settings/history writes have not undergone
forced-crash acceptance testing. Cancellation requests wait for terminal stream events;
quitting the process abruptly may lose an in-flight partial response. There is no
long-term memory extraction, file attachment, tool calling or background inference.

## Manual setup

In desktop Settings, choose a data folder, save the 9router base URL and optional API
key, click Save connection & load models (saves the entered values before testing;
a failed test does not roll them back), select/save a model, then send a non-sensitive
test message. Test stop-generation and reopen the conversation after restart. Do not
paste credentials into chat. A locally routed endpoint may still use remote inference.
