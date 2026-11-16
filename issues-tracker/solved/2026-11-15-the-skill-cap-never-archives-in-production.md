# The skill index cap never archives in production

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-runtime` |

## Observed

The only code that archives a learned skill when the index is over its cap is
`build_index(true)`, reached only through `SkillLibrary::index()`. Prompt
assembly calls `read_index()`, which never archives.

`crates/hearth-runtime/src/skills.rs:213`:

```rust
/// The prompt skill index: active skills, capped at 300 tokens and 50 lines.
/// Learned skills that no longer fit are archived. User skills are read-only.
pub fn index(&self) -> Result<String, SkillError> {
    self.build_index(true)
}

/// A read-only version of the index for auditing; never archives.
pub fn read_index(&self) -> Result<String, SkillError> {
    self.build_index(false)
}
```

`crates/hearth-runtime/src/agent.rs:111`:

```rust
Some(skills) => match skills.read_index() {
```

A search for `.index()` across `crates/` finds call sites only in
`crates/hearth-runtime/tests/skills.rs:65`, `:143`, and `:280`. The automatic
lifecycle archives only on 90 days unused or a sub-half success rate after four
uses (`skill_review.rs`). So a learned skill pushed past the 50-entry or
300-token cap is dropped from the prompt on every turn, and nothing archives it.

The tiebreak inside `build_index` is `left.name.cmp(&right.name)`
(`skills.rs:236`), not last use. `docs/DECISIONS.md:872` says learned skills over
the cap are archived "by lowest success rate, then least recently used", so the
second half is unimplemented too.

## Consequence

The learned library grows without bound, and which skills the agent sees is
decided by sort order and silent truncation rather than the documented retention
rule. The passing test at `tests/skills.rs:143` proves the behavior exists while
never being reachable from the daemon. SPEC 9.6 requires the index cap to archive
the overflow.

## Verification

Repeat `the_index_archives_low_success_learned_skills_over_the_cap` but drive it
through `Agent` prompt assembly instead of calling `library.index()` directly,
and assert that a learned skill ends up `Archived`. Today the prompt index is
shortened and the skill stays active.

## Resolution

`read_index` and the non-archiving branch of `build_index` are gone. `index()` is
the one builder, it always archives the overflow, and prompt assembly calls it.
The tiebreak now uses the documented order: success rate, then least recently
used, then name.

Commit: `9f1f831`. Verified by `the_prompt_index_archives_the_overflow`, which
drives a submission through `Agent` with 60 learned skills and requires at least
one archived; before the fix the prompt shortened the index and archived nothing.
