# Local recoverable text edits

## Current scope

Agent controls offer local Prepare, Apply, Undo, Discard and recovery-status inspection.
These are user-operated native commands, not AI edit tools. Existing chat tools remain
read-only. Provider sharing is separate; this editor does not upload replacements.

Only existing local regular UTF-8 files up to 1 MiB are supported. NUL/unsupported
encoding, alternate streams, restricted locations, links/reparse points, hard-linked
files and unsupported Windows attributes are rejected. Preserve BOM presence and CRLF
convention. No create/delete/rename, command execution or Unity mutation is implemented.
The UI previews the entered replacement, not an original-versus-replacement diff.

## Authorization and conflict checks

Prepare captures exact original bytes, grant ID/generation, conversation and task revision.
Apply rechecks current native Agent scope and permissions and compares full bytes, not
just timestamps or lengths. Task changes and grant changes invalidate pending proposals.
Proposals are memory-only, limited to eight, and expire after fifteen minutes.

Local mutation requires idle chat and holds its activity lock while performing the edit;
mode changes recheck that same lock at persistence. Settings changes serialize with Apply.
Stop uses a generation counter so requests made while work is queued are not cleared.
Cancellation is honored before mutation. Once a write begins it is completed or reported
uncertain, never described as automatically rolled back.

## Windows write boundary and recovery

Directory handles prohibit rename while held. The target is opened exclusively, checked
for reparse points/hard links/unsupported attributes and verified by its final handle path.
The target is written in place through that handle, synced, and read back for verification.
This is NOT a crash-atomic replacement: interruption can leave partial target contents.
This boundary is not a general sandbox or protection against a hostile local administrator.

Versioned prepared/committed/undoPrepared/undone records retain before/after bytes under
the selected Yuki data root. Record contents and stage consistency are validated. A failed
or ambiguous operation retains records; no automatic replay or destructive reconciliation.
Undo requires the current target to equal the recorded after-image and current task/grant
to match; external changes are never silently overwritten. Revoked grants may prevent Undo;
recovery material remains for review rather than bypassing permission revocation.

A 64 MiB logical quota reserves four records per change before mutation. This does not
reserve physical disk capacity or guarantee survival of power loss. Backups are not evicted
automatically. Corrupt/unknown records block normal recovery inventory and require manual
review; the UI does not implement forced recovery or backup cleanup.

## Verification limits

Automated tests use isolated fixture files and cover apply/undo, external conflicts,
task/grant invalidation, hard links, exclusive handles, recovery errors/corruption/quota,
and cancellation. Forced power-loss, hostile filesystem races and real-user-file acceptance
have not been demonstrated. Use disposable fixtures for the first desktop acceptance test.
