# The operator docs describe a CLI, key map, and memory design that are gone

| | |
| --- | --- |
| Kind | chore |
| Reported | 2026-11-15 |
| Area | `docs` |

## Requested

Correct the operator-facing documents so a new user does not follow instructions
that the binary does not support. The claims below are checkable in the tree and
each is false.

## The claims

- `QUICKSTART.md:122`: "Use `Tab`/`Shift+Tab` or `1`..`8` to move". The tab list
  has seven entries (`crates/hearth-cli/src/app.rs:43`, `Tab::ALL` has length 7)
  and the key handler matches `'1'..='7'` (`app.rs:417` and `:457`). Pressing
  `8` does nothing.

- `QUICKSTART.md:112`: "Skills browses the skill library read-only." The same
  file at `:55` says Skills "can create or edit your own skills", and the key
  handler binds `n` for a new user skill and `e` to edit one
  (`crates/hearth-cli/src/app.rs:859`, `ui/mod.rs:172`). The tab is not read-only.

- `README.md:82`: the `hearth-cli` row says "The `hearth` binary and its
  subcommands". `crates/hearth-cli/src/main.rs:24` accepts only `--version` and
  `--help`; any other argument prints "Run `hearth` with no arguments to open
  Hearth." and exits 2. `AGENTS.md:126` states the only user-facing entry is
  `hearth`, which opens the TUI.

- `README.md:73` describes `hearth-memory` as "Memory store, dedupe, standing
  memory, recall, extraction, consolidation, episodes". The current crate holds
  statements, rules, the retrieval cascade, the digest, and four tools
  (`AGENTS.md` architecture table). Standing memory, model extraction, and
  episodes are the pre-0.24 design. `README.md:72` also attributes "memory
  records" to `hearth-core`; the record lives in `hearth-memory` and the
  `memories` table.

## Why it is worth doing

These are the first documents a new operator reads. Each false claim sends the
reader to a surface that does not exist or away from one that does, and the
architecture table misleads the next engineer about where memory work belongs.

## Verification

Compare each quoted line with the cited source line. The key map and the
`--version`/`--help` behavior are observable by running the binary; the
architecture rows are checkable against `AGENTS.md` and `crates/hearth-memory`.

## Resolution

QUICKSTART now says `1`..`7`, describes Skills as browsing learned skills and
creating or editing user skills, and drops the turn limit from the Overview
list. README's architecture table no longer credits `hearth-core` with memory
records, describes `hearth-memory` as statements, rules, the retrieval cascade,
the digest, and the memory tools, and describes `hearth-cli` as the binary and
terminal application.

Commit: `2705eeb`. Verified against `Tab::ALL` (seven entries), the `1`..`7`
key ranges in `app.rs`, `main.rs` argument handling, and the `hearth-memory`
module list.
