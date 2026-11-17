# An xlsx shared-string table bypasses the document entry cap

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-tools` |

## Observed

`documents::archive_text` bounds one archive entry with `MAX_ENTRY_BYTES`. It
checks the declared size and then reads through `take(cap + 1)`, so a lying
header is still bounded.

`xlsx_text` does not use that helper for `xl/sharedStrings.xml`. It opens the
entry with `archive.by_name`, calls `read_to_string` on it, and passes the whole
string to `shared_strings`. A small deflated entry can expand to gigabytes, and
the read has no declared-size check and no take limit. The solved
`document-reads-have-no-size-cap` issue added the per-entry cap but missed this
path.

`pptx_text` and `xlsx_text` also concatenate every capped slide or sheet into one
`String` with no total bound. A file with many 16 MiB entries can still exhaust
memory, even though each individual entry is accepted.

## Verification

Build an xlsx whose `xl/sharedStrings.xml` is a few billion `A` characters,
deflate it, place it in the workspace, and call `documents` with `extract`. The
process allocates the expanded string rather than refusing at `MAX_ENTRY_BYTES`.
The existing `an_expanding_archive_is_refused` test covers `word/document.xml`
and a single sheet, not shared strings or the sum of entries.

## Resolution

`xlsx_text` reads `xl/sharedStrings.xml` through `archive_text`, so the entry
gets the same declared-size and take-limit checks as every other entry. Both
`pptx_text` and `xlsx_text` now append through `append_text`, and the tool
checks the final text, against a 32 MiB total cap
(`MAX_DOCUMENT_TEXT_BYTES`).

Commits: `d48eeb4` (test), `8973801` (fix). Verified by
`an_expanding_shared_string_table_is_refused` and
`a_document_with_too_much_text_is_refused`, which use 17 MiB shared strings and
three 11 MiB slides in a deflated archive.
