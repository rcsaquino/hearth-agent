# A draft with no successful use can be promoted to active

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-runtime` |

## Observed

The promote guard rejects only a non-draft with zero successes. A draft with zero
successes passes, which is the case the guard exists to block.

`crates/hearth-runtime/src/skill_review.rs:347`:

```rust
"promote" => {
    let skill = self.learned(name)?;
    if skill.successes == 0 && skill.status != SkillStatus::Draft {
        return Err("promote needs a successful use".to_owned());
    }
    self.skills
        .set_status(name, SkillStatus::Active)
```

The condition is the negation of its own error message: `successes == 0` is true
for a new skill, and `status != Draft` is false for a new skill, so the whole
condition is false and the promotion applies. `save_learned` creates every new
skill as a draft with zero counters (`skills.rs:378`).

The automatic lifecycle already promotes a draft when `successes > 0`, so the
model path is the only way an unvalidated draft reaches the prompt index.

## Consequence

A never-used procedure occupies one of the 50 index slots and part of the 300
index tokens that the agent reads every turn, with no evidence behind it. SPEC
9.6 says "A draft becomes `active` after one successful use", and the module's
own instruction says the same.

## Verification

Write a learned draft with `uses: 0`, `successes: 0`. Script a skill review
response of `{"action":"promote","name":"<name>","evidence":["entry-1"]}` with
one seeded successful run, and assert `report.rejected == 1`. Today the change
is applied. `cargo test -p hearth-runtime --test s7_skill_review` covers create,
revise, and the automatic lifecycle, not this branch.

## Resolution

The promote branch now requires `successes > 0` unconditionally. A draft that has
never been used has no evidence behind it, whatever status the file carries.

Commit: `6fc9179`. Verified by `a_review_cannot_promote_an_unused_draft`, which
cites a successful run that loaded another skill so the draft's counters stay at
zero; it was rejected zero times before the fix.
