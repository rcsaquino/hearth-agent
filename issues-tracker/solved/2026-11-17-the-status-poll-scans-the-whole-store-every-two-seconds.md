# The status poll scans the whole store and can archive skills

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-runtime`, `hearth-cli` |

## Observed

The TUI's `Workers::spawn_status` probes the daemon every two seconds through
`daemon_client::probe`. Each probe runs `ipc::status_value` on the daemon.

That function calls `Session::usage_totals_since(0)` for the lifetime total,
`MemoryService::count` for the current memory count, `MemoryService::rules` to
render every rule, and `SkillLibrary::index` to build the skills index. The
lifetime usage query scans every usage row. The memory count evaluates the
validity window over every memory row. The skills index reads and parses every
skill file. None of these are cached or aggregated between probes.

`SkillLibrary::index` is not a read. When more than `INDEX_MAX_ACTIVE` learned
skills are active, it calls `archive_for_index` and writes a skill file and a
changelog line. A status request can therefore mutate the skill library, and the
TUI can cause it while merely sitting on the Overview tab. `Agent::skills_index`
calls the same method on every turn, so the prompt path also performs this write
while building a request.

## Consequence

A long-lived TUI keeps a constant full-table scan and file read load on the
daemon. At a large store this is paid every two seconds for numbers that change
slowly. A read endpoint also has a write side effect that a caller cannot see.

## Verification

Instrument `usage_totals_since`, `store::count`, and `SkillLibrary::load_all`, or
watch the DB read counters for ten seconds with the TUI open and no messages.
Create more than 50 learned skills and select the Overview; the status poll
archives the excess and appends to `skills/learned/CHANGELOG.md`.

## Resolution

`StatusContext` keeps the lifetime usage total, memory count, rules, skills
index and stats, and the two review meta values for five seconds, so a two-second
probe no longer repeats those full scans. `SkillLibrary::index` is now a pure
view: past the 50-line cap it stops adding lines and never archives a skill. The
nightly review remains the writer.

Commits: `6053697` (test), `b2cb74b` (fix). Verified by
`the_index_bounds_the_view_without_archiving`, which builds 60 learned skills,
requires 50 index lines and zero archived skills, and by the existing
`ipc_status` suite.
