# Local workspace storage

## Implemented slice

Settings stores a data-root pointer and approved project roots under the Windows
app-config directory. Native folder selection creates a dedicated Yuki data folder
and checks writability. D: is a preference, not an assumed available drive.
Changing an existing nonempty or unavailable data root is rejected; safe migration
is not implemented. Cancellation retains settings. No original project files are deleted.

Projects supports explicit folder approval, revocation and on-demand immediate child
listing. Native commands recheck approved canonical roots, traversal and system/secret
restrictions. Unsafe links are rejected; listings are capped at 500 entries. Files
are listed only: content preview, indexing, editing and remote uploads are absent.
Browser preview cannot invoke native folder operations.

## Validation and limits

15 native helper tests and 6 frontend tests cover persistence, boundaries, navigation,
error handling and browser isolation. Native release compilation passed. Real native
picker interaction, installed desktop restart and forced-crash recovery have not been
manually exercised. Settings replacement uses a backup/rollback rename sequence with
backup recovery, not a single crash-atomic Windows replace operation. Concurrent
external settings writers are unsupported. Do not promise full migration/recovery guarantees.

Next: manual folder-picker/restart review, then define 9router connection acceptance.
