# One malformed skill file empties the whole skill layer

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-runtime` |

## Observed

A single unparseable skill file aborts the entire library scan. The prompt skill
index then becomes empty for every turn, and the `skill` tool cannot load any
skill.

`crates/hearth-runtime/src/skills.rs:164`, `load_all`:

```rust
pub fn load_all(&self) -> Result<Vec<Skill>, SkillError> {
    let mut skills = Vec::new();
    ...
                skills.push(parse_file(&file, SkillScope::User, false)?);
    ...
                skills.push(parse_file(&path, SkillScope::Learned, true)?);
```

The `?` aborts the loop on the first bad file. `parse_fields` errors on any
frontmatter line without a `:` (`skills.rs:749`), so an ordinary YAML block list
in a hand-written user skill fails the whole directory.

Every consumer inherits the error. `crates/hearth-runtime/src/agent.rs:108`:

```rust
fn skills_index(&self) -> String {
    match &self.skills {
        Some(skills) => match skills.read_index() {
            Ok(index) => index,
            Err(error) => {
                tracing::warn!(%error, "skill index could not be built");
                String::new()
            }
```

`find` also calls `load_all` (`skills.rs:282`), so `SkillTool` load fails for
every skill.

## Consequence

A user typo in one file disables all skills for the agent until the file is
fixed. The only trace is one `warn` log line and a Skills tab that lists
nothing. The per-file rejection is tested and deliberate
(`crates/hearth-runtime/tests/skills.rs:98`); the global blast radius is not.

## Verification

Write one valid user skill and one learned skill with a frontmatter line that
has no colon, then call `read_index()` and `find(valid_name)`. Both fail. The
expected behavior is that the valid skills remain available while the bad file is
reported as a per-file warning.

## Resolution

`load_all` parses one file at a time and skips a malformed file with a
`tracing::warn` naming the path. The valid skills stay in the index and in
`find`.

Commit: `9f1f831`. Verified by `one_malformed_skill_does_not_hide_the_rest`,
which writes one good user skill and one broken learned skill and requires the
good one to load.
