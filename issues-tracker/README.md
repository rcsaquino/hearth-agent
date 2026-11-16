# issues-tracker

The issue tracker for the Hearth harness.

It is the queue. An issue is anything concerning the repository that someone
has decided is worth recording: a fault, a missing feature, a rough edge, a
chore. It is not only for bugs.

Entries are Markdown, one per file. The directory a file sits in is its status.
Nothing else tracks it.

## Layout

| Directory | Meaning |
| --- | --- |
| `open/` | Recorded, not yet dealt with. This is the work queue. |
| `solved/` | Work landed and was verified. The file records what changed. |
| `closed/` | Closed without work: declined, not reproducible, already exists, duplicate, or superseded. The file says which. |

The directory listing is the index. Do not keep a separate list of files.

## Kinds

Every file declares its kind in the header. The kind decides what "solved"
means, so get it right before writing anything else.

| Kind | What it is | Solved means |
| --- | --- | --- |
| `bug` | Something does not work as intended. | The mechanism is gone from the code. |
| `feature` | Something the repository does not do and should. | It exists, is reachable from the documented surface, and is tested. |
| `change` | Something works but should work differently. | The behaviour is different and tested. |
| `chore` | Upkeep: dependencies, docs, layout, tooling, build. | The upkeep is done. |

If the kind is wrong, change it and say so in the file. A bug filed as a feature
will be implemented rather than fixed, and nobody will notice.

## Header

Start every file with the same block.

```markdown
| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | hearth-memory |
```

`Area` is the crate or directory most involved. Files written before this block
existed are bugs by default.

Names are `YYYY-MM-DD-short-slug.md`, using the date the issue was written. The
slug describes the subject, not the kind; the kind belongs in the header where
it can be read without decoding a filename.

## If you are filing an issue

Write it to `open/`.

Write for an agent that has only the repository. It cannot see the
conversation, the host, the logs, or the person who noticed the problem. If a
claim cannot be re-derived from code in the repository, it does not belong in
the file.

For a `bug`, state the observed behaviour and the mechanism. Point at files and
functions. Do not propose a fix; the file describes the fault, the fixer decides
the approach.

For a `feature` or `change`, state what should happen and why it is worth doing.
Describe the desired behaviour, not the implementation. Leave the design to the
implementer, and say plainly if there is a design constraint that must hold.

Do not paste secrets, tokens, keys, credentials, or message identifiers.

If you are unsure whether something is worth filing, file it and say what is not
established. An issue can be closed. A forgotten one cannot be worked on.

## If you are working an issue

1. Read `open/`.
2. Establish the current state. For a `bug`, reproduce it or show from the code
   that it cannot still occur. For a `feature` or `change`, show that it does
   not already exist.
3. Do the work.
4. Move the file to `solved/` and append a **Resolution** section: what changed,
   the commit, and how it was verified.

### Moving to solved

If only part of the issue is addressed, say so in the Resolution section. Split
the remainder into a new file in `open/` rather than leaving a solved file that
is half true.

### Moving to closed

Append a **Closure** section: the reason, in one of these forms.

- *Declined* — considered and not adopted, with the reasoning that decided it.
- *Already exists* — the repository already does this; say where.
- *Not reproducible* — could not reproduce, with what was tried.
- *Working as intended* — the behaviour is deliberate; cite the decision or test
  that encodes it.
- *Duplicate* — name the file it duplicates, and merge any detail it added.
- *Superseded* — a later change removed the affected code or made the request
  moot.

A bug closed as intended behaviour should keep its reasoning. If a test asserts
the behaviour is correct, cite the test. That is what converts a bug into a
decision, and the next reader needs to know it was deliberate.

## Rules

- Never delete an issue. Closed and solved files are the record.
- Never move a file back to `open/` silently. If a solved fix regressed, file a
  new issue and reference the old one.
- One issue per file. Two unrelated things in one file means one of them gets
  lost.
- Update the file, not a chat message. Anything a worker needs must be in the
  file.
- Ordering inside a directory does not matter. Nothing is by priority unless a
  file says so in its own text.
