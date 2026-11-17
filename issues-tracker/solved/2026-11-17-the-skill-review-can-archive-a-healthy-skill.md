# The skill review can archive a healthy skill without meeting the archive rule

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-runtime` |

## Observed

`SkillReviewer::apply_change` enforces the stated conditions for create,
promote, and quarantine. Its `"archive"` branch only calls `self.learned(name)?`
and `SkillLibrary::set_status(Archived)`. It does not check that the skill has
been unused for 90 days or has a failure rate above half after four uses. The
instruction tells the model those are the conditions, and the automatic
`apply_lifecycle` applies them, but a model-supplied change is not checked.

Any change object with an action of `archive`, a learned skill name, and one
successful entry id can archive an active skill with one use. The model may be
mistaken, or a prompt injection in a reviewed entry may ask for it. The result is
removed from the skills index and is only recoverable by editing the file.

## Verification

Use a scripted provider that returns an `archive` change for a new learned skill
with one successful use. `SkillReviewer::run` returns `applied = 1`, and the
skill status is `archived`. `apply_lifecycle` alone would leave it active.

## Resolution

The `archive` branch of `SkillReviewer::apply_change` now applies the same
condition as `apply_lifecycle`: at least 90 unused days, or a failure rate above
half after four uses. A model-supplied archive for a healthy skill is rejected.

Commits: `6a973a1` (test), `d4857c9` (fix). Verified by
`a_review_cannot_archive_a_healthy_skill`, which archives an active skill with
one successful use and requires a rejection with the status unchanged.
