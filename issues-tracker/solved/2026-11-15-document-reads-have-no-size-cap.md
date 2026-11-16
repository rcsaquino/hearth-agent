# Document and file reads have no size bound

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-tools` |

## Observed

The document tool decompresses archive entries with no limit, and the file tool
reads a whole file into memory before slicing lines. A small compressed document
can expand to gigabytes.

`crates/hearth-tools/src/documents.rs:168`:

```rust
fn archive_text(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<String, ToolError> {
    let mut file = archive
        .by_name(name)
        .map_err(|_| ToolError::invalid("corrupt_document", format!("missing {name}")))?;
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(|error| {
```

There is no `Read::take` and no check against the entry's uncompressed size.
`documents.rs:121` reads the compressed file first with
`self.workspace.read_bytes(path)?`. The entries read this way include
`word/document.xml`, `xl/sharedStrings.xml`, and sheet XML, where deflate reaches
roughly 1000:1.

`crates/hearth-tools/src/workspace.rs:65`, `read_text`:

```rust
pub fn read_text(&self, path: &str) -> Result<String, ToolError> {
    let target = self.resolve_read(path)?;
    // ...
    fs::read_to_string(&target).map_err(ToolError::from)
}
```

`files read` caps output at 2000 lines, but only after `read_text` has the whole
file resident (`crates/hearth-tools/src/file_tool.rs:62`). Inbound media is
capped at 20 MB on download, which bounds the compressed input and nothing about
the expansion; a local file reached through `files read` has no cap at all.

## Consequence

A `.docx` or `.xlsx` of a few megabytes expands to gigabytes and the kernel kills
the daemon mid-run. The run is recovered on restart, so this is a crash rather
than data loss. A large local text file through `files read` produces the same
effect without an archive.

## Verification

Build a docx whose `word/document.xml` is a few billion `A` characters, deflate
it, place it under the workspace or inbound media, and call `documents extract`.
Watch process memory. The expected behavior is a refusal at a documented cap,
not an allocation proportional to the uncompressed entry.

## Resolution

Three caps, each with a named constant: `workspace::MAX_TEXT_BYTES` (16 MiB) for
a text read, `workspace::MAX_DOCUMENT_BYTES` (32 MiB) for an archive read, and
`documents::MAX_ENTRY_BYTES` (16 MiB) for one uncompressed entry. The entry check
reads its declared size first and then reads through `take(cap + 1)`, so a lying
header is still bounded.

Commits: `a9a8cca` (test), `9477cbc` (fix). Verified by
`an_expanding_archive_is_refused` and `a_huge_text_file_is_refused`, both of
which returned content or a different error on the parent commit.
