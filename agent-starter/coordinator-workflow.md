# Yuki coordinator and subagent workflow

Use this playbook to continue development with the same coordinator-led workflow.
It is project guidance, not permission to override host policies or user intent.
Read the actual files and verify tool/model availability; do not imitate tools that
are unavailable or assume a model name proves how an external provider routed it.

## Start prompt for another agent

> Work as Yuki's coordinating engineering agent. Read `AGENTS.md`,
> `agent-starter/AGENTS.md`, `agent-starter/engineering.md`, and
> `agent-starter/coordinator-workflow.md`. Follow their relevant references.
> Read `docs/README.md` and the latest section of `docs/handoff.md`, then verify
> Git state and the actual implementation. Summarize current scope and blockers.
> Use Research for substantive research, Coding for bounded native implementation,
> and Hard UI for UI work. Coordinate independent ownership, integrate and validate
> the results yourself. Implement only the approved next slice. Preserve user work,
> report actual evidence, update the handoff, and publish only under the current
> repository authorization. No assistant attribution or co-author commit trailers.

Paths in that prompt are relative to the repository root. A Markdown playbook is
not automatically loaded by every agent host: explicitly reference it when handing off.

## 1. Reload context before acting

1. Read the [root rules](../AGENTS.md), [starter entry point](AGENTS.md), and
   [engineering workflow](engineering.md), including applicable nested instructions.
2. Read the [documentation map](../docs/README.md), the latest
   [handoff](../docs/handoff.md), and only relevant topic documents.
3. Before files, memory, providers, tools or integrations, read
   [assistant safety](assistant-safety.md).
4. Check branch, working tree and task-owned diffs. Treat existing changes as owned
   by someone else unless their provenance is known. Never revert them casually.
5. Separate implemented behavior, user-accepted behavior, approved future work and
   unresolved proposals. Tests are not visual approval or live-provider acceptance.

The handoff is the current state source, not this playbook. Historical entries can
contain superseded limitations. Verify the latest summary against code and Git.

## 2. Settle decisions, then implement one slice

- Reuse settled decisions. Do not re-grill a visual direction or permission policy
  merely because the session changed or a worker forgot it.
- For decision-critical ambiguity, use the host's structured question tool. Collect
  independent questions in one round; defer questions that depend on unanswered ones.
- Give recommendations and trade-offs. Environment facts are the agent's job to find.
- During grilling, do not implement until the user confirms shared understanding.
- Once scope is approved, proceed without repeatedly requesting the same approval.
  A new capability, changed permission or destructive operation needs its own gate.
- Define an observable acceptance condition and bounded stop for every slice.

## 3. Research before non-trivial choices

Delegate substantive research to Research. Investigate root causes and compare
credible alternatives instead of implementing the first plausible idea. Prefer
current primary documentation and source at a concrete version. Request citations,
compatibility, maintenance/licensing, safety implications and unresolved uncertainty.

Stop when evidence supports a decision; reopen research when validation contradicts
it. Simple known-file/documentation edits do not need an artificial research project.
Research does not authorize uploading private code, inspecting personal configuration,
installing software, launching processes or expanding the task scope.

For bugs, establish a reproducible failing signal before claiming a root cause.
Public documentation alone is not evidence that the user's installed integration works.

## 4. Route work to the required models

| Work | Required selection | Coordinator responsibility |
| --- | --- | --- |
| Research | Research: `9router-github-copilot/research` | Integrate evidence and check applicability |
| UI design/implementation | Hard UI: `9router-github-copilot/hard-UI` | Preserve visual direction; integrate and validate |
| Bounded native/general coding | Coding: `9router-github-copilot/coding` when exposed | Define contracts and review implementation |

The [root rules](../AGENTS.md) and [engineering model preferences](engineering.md)
are authoritative if revised. Use an explicit model override when required by those
preferences; otherwise leave runtime defaults alone. Agent type and model are different:
a Research agent dispatched on Planning is not the requested Research model.

If the required model is unavailable, report the actual error. Do not silently choose
another model. For Coding failures, obtain approval before a different execution approach
unless current instructions already authorize it. A prior task-specific fallback is not
a permanent grant. Never treat a worker's unsupported self-identification as definitive
provider evidence; report conflicts with runtime metadata honestly.

## 5. Delegate with complete, bounded briefs

Every worker brief includes:

- Objective and confirmed acceptance criteria.
- Relevant rules, source files, contracts and decisions to read first.
- Exact files/modules it owns and files it must not touch.
- Known dirty work, parallel workers and shared integration points to preserve.
- Privacy, permission, dependency and side-effect restrictions.
- Required targeted tests/builds, expected return contract and stop condition.
- No nested delegation or commits/pushes unless specifically assigned.

Ask for implementation and actual evidence, not merely suggestions, when implementation
is approved. Require changed paths, exact IPC/event contracts, test results and limitations.
Do not let workers claim unregistered tests ran or substitute a build for runtime testing.

## 6. Parallelize independent work, not shared state

Good pipeline:

1. Research establishes a version-specific contract.
2. Coding implements the native contract and isolated fixtures.
3. Hard UI integrates the stable contract while the coordinator validates native work.
4. Coordinator fixes integration issues and runs combined gates.

Independent features may progress together when explicitly approved, such as Notes
storage and MCP discovery. Keep ownership disjoint. Serialize shared files such as
`lib.rs`, `App.tsx`, global styles, dependency manifests and the handoff, or reserve them
for the coordinator. Do not start unrelated features just to keep workers occupied.

While a background worker runs, do genuinely independent work. Do not duplicate its
searches or edit its owned files. Keep its returned agent ID. Wait for notification,
then read its results; avoid repeated status polling or listing agents to rediscover IDs.
Use follow-up messaging only for the same retained-context task.

## 7. Recover from tool/model failures without spam

- Stop repeating an identical malformed or failing tool call. Fix its arguments once,
  or report that the host interface is blocking progress.
- After an agent/provider failure, inspect Git/files before retrying. Preserve partial
  work, distinguish it from user changes, and resume from the actual state.
- Do not repeatedly relaunch an empty/failed worker. A user-approved retry or a genuinely
  narrower completion task is different from an uncontrolled retry loop.
- A tool-count warning means too many enabled tools, not missing dependencies.
  Installing more tools does not fix it. Do not alter user configuration without consent.
- Do not reinstall dependencies unless manifests changed or validation reports a missing
  dependency. Do not launch a real relay/provider or inspect personal files when scope
  permits fixtures only.

## 8. Integrate and verify as coordinator

Review worker reports and inspect changed interfaces/boundaries. Do not accept a green
report blindly. Check stale responses, cancellation, revocation, failure preservation,
encoding/size bounds, privacy and legacy data compatibility where affected.

Use the smallest meaningful checks first, then affected integration gates. Typical Yuki
commands, from the repository root, are:

```powershell
npm test -- src\Relevant.test.tsx
npm run build
# Native checks start inside src-tauri:
Set-Location src-tauri
cargo test relevant_filter
cargo fmt --check
# A separate shell call starts at repository root again:
npm run tauri build -- --no-bundle
```

Run full suites when shared contracts or combined changes warrant them. Documentation-only
work needs link/format checks, not an application rebuild. On Windows use PowerShell-safe
commands and explicit exit-code checks; avoid unsupported shell operators. Tool processes
may not retain working directories or environment variables between calls.

Read completion output from the original background shell rather than rerunning a build.
Stop task-owned helpers by exact process/session identity; do not kill processes by name.
Use disposable data for mutation tests. Passing fixtures do not prove real-user acceptance,
crash atomicity, perfect secret detection or live model compatibility.

## 9. Keep the UI consistent

Hard UI must read the [approved visual direction](../docs/product/visual-direction.md).
Reuse shared typography, palette, controls, focus/motion behavior and status language.
Preserve accepted screens instead of redesigning them on every slice. Specialized
surfaces such as notebook Notes must still belong to the same system.

Require readable sizing, desktop/narrow layouts, keyboard usability, reduced motion,
and truthful progress/error states. Report when rendered/native inspection was not done.
A user saying the overall look is good is feedback to preserve, not proof every control
or future screen has been accepted.

## 10. Checkpoint, deliver and publish carefully

The coordinator maintains one concise latest summary in the existing handoff: approved
scope, ownership, changed paths, actual checks, blockers, acceptance and next action.
Link architecture/product details rather than accumulating duplicate transcripts. Update
directly affected docs. Do not make extra planning journals unless requested.

Before committing, review the diff, stage only task files, check the staged diff and
use the configured identity. Follow current root/user authorization for commit/push;
for this repository focused verified publication to `dev` is authorized. Never add
assistant credit or `Co-authored-by` trailers. Never amend or force-push without explicit
permission. On divergence, inspect history and ask before rewriting published history;
use an exact force-with-lease only when authorized.

Report commit IDs, remote verification, checks and remaining limitations concisely.
Do not announce completion while a required build is still running or a failure remains.
For staged work, explicitly say which boundary is complete and which remains disabled.

## Current continuation pointer

Consult the latest [handoff](../docs/handoff.md) before proceeding. At the time this
playbook was written, guarded user-operated local text edits were published as `db9447a`;
chat editing was still read-only. The next approved stage was AI edit integration under
standing write grants, not arbitrary shell or Unity mutation. This paragraph is a dated
orientation, not a replacement for live repository state or renewed scope checks.
