# User-authored guidance is truncated instead of read whole

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | `hearth-core` |

## Current

`HEARTH.md` and every `AGENTS.md` are cut at read time, at two limits in
`crates/hearth-core/src/guidance.rs`.

- `MAX_GUIDANCE_FILE_BYTES` is 16 KB. `read_guidance` reads with a capped take,
  truncates on a char boundary, and appends `[truncated at 16 KB]` to the text.
- `MAX_GUIDANCE_TOTAL_BYTES` is 32 KB. In `load`, the first file that does not
  fit is cut mid-text with `truncate_to`, `[guidance truncated at 32 KB]` is
  appended, and the source loop breaks, so every later file is dropped with no
  marker.

Sources are ordered home `HEARTH.md` first, then `AGENTS.md` from the filesystem
root down to the workspace, in `load`. Because the total limit breaks the loop,
the file dropped is always the most specific one. SPEC 3.1 says the files are
read "most specific last"; the last is the one that does not survive.

The cut is not visible anywhere except inside the prompt. `Guidance.truncated`
is set by `load` and read only by the `s1_guidance` tests; no runtime, TUI, or
log surface consumes it. `TokenCounts::over_budget` in
`crates/hearth-core/src/budget.rs` does not list `guidance`, so a guidance
overrun raises no warning either.

`SOUL.md` is not cut today: `Daemon::build` in
`crates/hearth-runtime/src/daemon.rs` reads it with `read_to_string`. It is
counted against the 800-token L0 identity budget, which produces one
`tracing::warn` in `Agent::run` and nothing else. The two file kinds are
therefore already inconsistent, and the bundled file is the unbounded one.

The cap is close in practice: the repository's own `AGENTS.md` is 12,014 bytes
against a 16,384-byte limit.

## Desired

These files are read whole and placed in the prompt as written. Neither the
engine nor the TUI refuses a write because of length. A person who writes 40 KB
of standing instruction gets 40 KB of standing instruction.

The cost stays visible. Segment token counts are display-only today — `guidance`
and `fixed` are serialized by `prompt::build` and no code branches on them — so
the count should remain and be reported, so a file that has quietly grown to a
hundred kilobytes is obvious before it becomes a failure.

The model window is a physical limit and cannot be removed. When the guidance
alone no longer leaves room for a turn, the outcome should be the existing
explicit failure path in `Agent::run`, "The conversation is too large for the
model window...", with the offending file and its size named, rather than a
silent cut. A named failure is debuggable; a truncated instruction is not.

## Why

These files are the person's own words about how they want the agent to behave,
and in this repository `AGENTS.md` is also how a project tells the agent what it
may not do. Dropping the tail of one is a silent behaviour change: the
instruction that vanished is precisely the one the person added last, and
nothing in the UI, the log, or the saved prompt says so. A file that is too
large costs tokens and is easy to notice in the Prompt tab. A file that was cut
looks normal and is not.

## Constraint

Guidance is in the cached prefix and is covered by the prefix hash, so editing a
large file invalidates the provider cache. That is a real cost, and it is the
person's to accept; it is not a reason to cut.

## Documented as intended

The caps are recorded as deliberate, so this change moves them together: SPEC
6.1 (L1 budget "32 KB text"), SPEC 3.1, the `AGENTS.md` invariants ("Each file is
capped at 16 KB and the total at 32 KB"), and the `Guidance.truncated` field and
its tests.

## Related

`2026-11-16-an-over-budget-prompt-segment-warns-nobody.md` — `Guidance.truncated`
is set but never shown, which is the same missing reporting from the other side.

## Verification

Write `HEARTH.md` and a workspace `AGENTS.md` larger than 16 KB each, with a
distinctive sentence in the last line of each. Assemble a prompt. Assert both
sentences are present, that no truncated marker appears, and that
`tokens.guidance` reports the full cost. Then set `models.context_window` low
enough that the guidance cannot fit and assert the turn fails naming the file.

## Resolution

`load_guidance` now reads every file whole. `MAX_GUIDANCE_FILE_BYTES`,
`MAX_GUIDANCE_TOTAL_BYTES`, `truncate_to`, the truncation markers, and
`Guidance.truncated` are gone. `Guidance.files` holds
`GuidanceFile { path, bytes }`, and `AgentConfig` carries the `Guidance` value
instead of a detached string, so the budget failure can name a source.

`Agent::budget_failure` keeps the existing message when the strain is elsewhere.
When `tokens.guidance` is at least the usable window, it names the largest
guidance file and its byte size, and the provider is never called. The prompt
still records `tokens.guidance`, and the Prompt tab shows that layer's count, so a
file that grows is visible.

Commits: `bafb740` (test), `3b853cd` (feat), `dcd61c7` (docs).

Verified by `cargo test -p hearth-core --test s1_guidance`:
`every_guidance_file_is_read_whole` and `the_total_guidance_is_not_capped` write
distinctive last lines into files over the old caps and assert both reach the
prompt with no truncation marker. `guidance_that_cannot_fit_names_its_file` in
`crates/hearth-runtime/tests/s6_agent.rs` writes a 100 KB `HEARTH.md`, runs a turn
against a small `ContextBudget`, and asserts the notice names the file and byte
size, with no provider call. `./scripts/ci.sh` passes.
