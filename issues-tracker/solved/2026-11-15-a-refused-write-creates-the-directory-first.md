# A refused write creates directories outside the workspace first

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-tools` |

## Observed

`resolve_write` creates the target's parent directory before it checks whether
the canonical parent is inside the workspace. A symlinked directory therefore
lets a refused write create a directory tree outside the root.

`crates/hearth-tools/src/workspace.rs:218`:

```rust
let root = fs::canonicalize(&self.root)?;
let target = root.join(&relative);
let parent = target
    .parent()
    .ok_or_else(|| ToolError::invalid("invalid_path", "path has no parent"))?;
fs::create_dir_all(parent)?;
let canonical_parent = fs::canonicalize(parent)?;
if !canonical_parent.starts_with(&root) {
    return Err(escape(path));
}
```

With `workspace/out` a symlink to `/tmp/elsewhere`, a call to write
`out/deep/x.txt` runs `create_dir_all` on `workspace/out/deep`, which follows the
symlink and creates `/tmp/elsewhere/deep`. The next line canonicalizes the parent,
finds it outside the root, and returns `path_escape`. No file content escapes,
but the directory tree is left behind with model-chosen names.

`docs/DECISIONS.md:208` states the opposite: "`..`, absolute paths, and symlink
escapes are rejected before anything is written." The existing test
`rejects_path_escapes` (`crates/hearth-tools/tests/tools.rs:58`) covers a
symlinked file and not a symlinked directory.

## Consequence

A refused call mutates the filesystem outside the workspace. The content remains
confined, so this is a broken boundary invariant rather than an escape of data.
A maintainer reading the decision will believe the guard runs before any
mutation.

## Verification

Create a directory outside the workspace, symlink it into the workspace as
`out`, and call `files write` with `out/deep/x.txt`. Assert the call returns
`path_escape` and the outside `deep` directory was not created. Today the error
is returned and the directory exists.

## Resolution

`resolve_write` canonicalizes the deepest existing ancestor of the target and
checks it against the root before calling `create_dir_all`. The post-check stays
as a second guard. A refused write now creates nothing.

Commits: `b48dde7` (test), `dae1c12` (fix). Verified by
`a_refused_write_creates_nothing_outside_the_workspace`, which symlinks a folder
outside the workspace, writes `out/deep/x.txt`, asserts `path_escape`, and
asserts the outside `deep` folder does not exist.
