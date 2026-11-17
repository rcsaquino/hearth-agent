# The documents table and conversation fork have no production caller

| | |
| --- | --- |
| Kind | chore |
| Reported | 2026-11-17 |
| Area | `hearth-core`, `hearth-storage` |

## Observed

`Session::fork` copies a conversation up to a chosen entry, gives it a new id,
and also copies every `documents` row. No production code calls `.fork(`. The
only callers are tests.

`Save::document`, `Session::document`, the `Document` record, and
`DocumentWrite` are the only way to read or write the `documents` table. No
runtime feature creates or reads a document. The `documents` table is written by
`Session::save`'s document branch, read by `Session::document`, copied by
`Session::fork`, and otherwise unused. The built-in kinds listed in
`docs/SCHEMA.md` are not written anywhere.

The conversation parent fields are set by `create_conversation_with_parent` for
helpers and by `fork`; no user-facing view reads them once fork is gone.

The crate module doc calls the crate "the permanent record, saves, documents,
submissions, and forks", and SPEC 4.3 presents documents as one of the six record
kinds. That is not the running system.

## Wanted

Either remove the fork and document subsystem, with a new migration that drops
`documents` (0001 cannot be edited), or give it a real caller and a documented
surface. Do not leave a schema, types, save branch, and tests for a feature no
code reaches. Remove the parent columns only if helper provenance has another
home.

## Verification

`grep -rn '\.fork('` and `grep -rn '\.document('` find only tests. The
`documents` table has no row in a normal home. `cargo test` exercises the
version-conflict rules because the tests call `Save::document` directly, not
because a product path does.

## Resolution

Removed the documents table from the running schema with migration 0011, and
removed Document, DocumentWrite, Save::document, Session::document,
Session::fork, and CoreError::DocumentVersion. The conversation parent columns
stay because helper conversations use them. The SPEC, SCHEMA, and decision log
now describe five record kinds and no fork.

Commit: `28b5f6a`. Verified by `the_retired_documents_table_is_gone` and the
existing core and storage suites, which no longer have document or fork tests.
