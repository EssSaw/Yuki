# Engineering Workflow

## Decisions and scope

- The user owns product scope and approvals. Ask focused rounds of related questions when behavior, compatibility, safety or architecture depends on a choice; recommend an option and explain its trade-off. Record confirmed decisions separately from test defaults.
- Plan long work in dependency-ordered, independently testable slices. Give each slice observable acceptance criteria. Preserve working behavior outside its scope; fix tightly coupled regressions, not unrelated debt.
- Features absent from a new specification are candidates for reuse/retirement, not automatically dead code. Trace source, assets/configuration, callers and persisted dependencies; retire only after disposition is confirmed. Git history is the archive, not a duplicate legacy implementation.

## Smallest correct implementation

1. Check whether the feature is necessary now.
2. Reuse existing types/helpers, then standard library/native platform features, then installed dependencies.
3. Only then write the minimum code that meets the approved behavior.

Prefer concrete, well-owned modules over generic frameworks. Separate stable definitions from mutable per-user/per-session state where the application actually needs both. Add extension seams for confirmed variations, not every possible future backend. Do not add single-implementation factories/interfaces or speculative configuration.

Read actual signatures, callers and configuration before editing. Fix shared causes rather than patching symptoms across consumers. Validate input at trust boundaries. Handle failures before mutating durable state or reporting success. Preserve cancellation, accessibility and privacy even when simplifying.

## Quality and performance

- Use consistent repository naming/style and mandatory braces where the language/conventions support them. Keep comments for non-obvious reasoning. Remove obsolete code rather than accumulating patches.
- Make state ownership, lifecycle, cancellation and cleanup explicit. Avoid leaked event subscriptions, background jobs or shared mutable request state. Keep UI rendering separate from domain/storage logic; views should subscribe to changes instead of unnecessary polling.
- Make units, clock source, timezone and numeric limits explicit when correctness depends on them. Avoid exact equality for approximate floating-point calculations; validate non-finite and out-of-range values.
- Profile representative workloads before optimizing. Bound context size, queues, concurrency and retries. Prefer batching/pagination and bounded streaming over unbounded loading. Do not impose a game-style zero-allocation target on every web/request path.
- Add dependencies only when justified by the task; install/restore after manifest changes or a missing-dependency validation failure. Preserve lockfiles and existing ecosystem tooling.

## Verification and delivery

- Non-trivial logic leaves one appropriately scoped runnable regression check; use existing test infrastructure, not a new framework by default.
- Run the smallest meaningful tests first, then affected build/typecheck/lint gates. Escalate when targeted evidence indicates wider impact. For code slices, the affected build must have no new warnings/errors; report any pre-existing baseline failures accurately. Documentation-only changes need link/format checks, not an application build unless docs tests exist.
- Use isolated temporary data and test accounts. Never run migrations/tests against real personal data without explicit approval. Test backup recovery, repeated migration, corrupt/future-version data and failed writes where persistence changes.
- Report what actually ran and its result. Distinguish contract/unit checks, integration tests, visual review and production acceptance; never substitute one for another or fabricate performance evidence.
- Preserve uncommitted work. Stage only task files if commits are authorized. No commit, push, force-push, deploy, destructive cleanup or external write without explicit authorization covering that action. A one-off permission does not become a permanent rule.
- Remove task-owned temporary scripts/artifacts after verification; retain intentional regression checks. Stop task-owned background services when no longer needed, using specific process/session identifiers.
- Finish with changed paths, concise rationale, validation evidence and unresolved limitations. Stop for user review unless an approved batch remains active.

## Tools, skills and delegation

Use relevant available skills; verify availability rather than relying on old installation snapshots. When a needed capability is missing, search online for the best compatible skill, preferring maintained, authoritative sources. Compare host/tool compatibility, permissions and dependencies; inspect its instructions and scripts before proposing installation. Do not transmit private project data during discovery or install/run untrusted code without approval. If no suitable skill/tool is available, name the capability and blocker; do not claim equivalent work was performed.

Handle bounded known-file work directly. For substantial coding and planning batches, split independent tasks and use multiple subagents wherever useful and supported by the host. Give each a precise scope, inputs, owned files/state, stop condition and acceptance checks. Parallelize only independent file/state scopes; serialize shared scenes, databases and other side effects. Verify returned evidence before claiming completion.

Current user-requested model preferences: `9router/coding` for coding subagents and `9router/planning` for planning subagents. Verify these exact IDs are exposed by the host before dispatch. If unavailable, report the limitation and obtain approval for a substitute; do not silently map them to another model or claim to have used them. These preferences are revisable and do not authorize bypassing host restrictions.

## Context continuity

Organize growing documentation hierarchically: keep entry points short, link to focused topic files, and load only the branches relevant to the current task. Keep each decision/rule in one authoritative location rather than duplicating it across summaries. Split a document when unrelated topics or repeated context make selective reading difficult; do not create empty scaffolding or split small cohesive files merely to meet a line count. Handoffs should link to detailed evidence instead of copying it wholesale.

Do not rely on chat history or a subagent's retained context as the only source of task state. Maintain one concise current handoff in an existing project planning/handoff artifact; if none exists, use session storage until a project artifact is authorized. Record the goal, confirmed decisions, unresolved questions, scope/ownership, changed paths, actual validation results and next action. Exclude secrets and unnecessary personal data.

Checkpoint before compaction, pause, task transfer and after significant decisions or completed slices. On resume, reread the starter rules and current handoff, then verify relevant files and working-tree state before proceeding. Supply each subagent with the applicable rules, relevant decision/source paths and a bounded task brief; require a return summary of changes, evidence and blockers, and reconcile it into the handoff. Explicitly report missing or stale context rather than guessing. This reduces context loss; it cannot guarantee unlimited memory or recovery of unsaved state.
