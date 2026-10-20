# Agent mode and grant foundation

## Implemented boundary

Chat and Agent are explicit conversation modes. Legacy conversations default to Chat.
Agent setup selects an approved project and optional task description; mode/root metadata
is not appended to provider message text. Existing read tools remain unchanged.

This slice does not implement project-file writes, command execution or Unity mutations.
The UI states that execution is unavailable. Saved grants are preparatory authorization
records, not a working executor or OS sandbox. No task resumes automatically on restart.

## Grants and state

Workspace settings own write-root grants, reviewed command-template records and a
monotonic generation. Grant changes require the expected generation; stale requests
fail. Reapproval creates a new UUID. Removing a project clears dependent Agent grants.
Provider-sharing and MCP approvals remain distinct and are not expanded by Agent mode.
Settings read/modify/write mutations use a shared process lock; multiple external
settings writers remain unsupported.

Write roots are canonical directories within approved projects, subject to restricted
path/link checks. Command records hold exact executable, arguments, working directory
and optional script fingerprint. Creation does not execute anything. Command-template
editing is not exposed in the first UI; full execution-time validation is future work.

## Interface and limits

Chat provides consistent mode/task controls and native effective permission snapshots,
write-root review/revocation, stale-response protection and active-request guards.
Browser preview cannot mutate native grants. New conversations reset mode setup;
reopened conversations load their saved mode without executing a task.

Future executors must recheck current mode, task identity, grants, paths, command/script
identity and cancellation at execution time. Current records and directory choice do
not constitute containment or a guarantee that arbitrary commands are safe.

See [approved policy](../product/scope.md#approved-staged-agent-capabilities-2026-10-20)
for fresh-approval actions, recovery requirements and the staged implementation order.
