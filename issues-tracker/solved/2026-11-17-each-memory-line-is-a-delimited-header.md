# Each memory line is one delimited header

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-17 |
| Area | `hearth-memory` |

## Observed

`digest::recall_body` joins one line per memory with a newline and
`build_prompt` wraps the body in a single `<memory>` system block. `digest::line`
renders `<id> <kind> <subject> <age>: <content>`. There is no legend: the block
names no column, and the prompt adds nothing around it.

The line has no separator between its fields:

- The kind is omitted for a fact, so the first token varies by kind.
- An empty subject or age is omitted, so the field count varies by row.
- The subject may be several words. In the reference home, 78 of 106 current
  memories have a multi-word subject across 23 distinct subjects, so most lines
  read like `or board now: ...`, where only the age alphabet tells the reader
  that the subject ends at `board`.
- `person`, `commit`, `episode`, and `pref` are ordinary words or truncations,
  with nothing marking them as a kind.

The open issue to remove the memory id from the digest
(`2026-11-17-the-memory-id-costs-tokens-on-every-turn.md`) takes away the 34
character first token, which was the strongest signal that the rest of the line
is metadata. The header has to carry that signal itself.

The newline is already a reliable row boundary: `clip` collapses whitespace, so a
statement can never contain one. The only other lines in the block are `more:`
and `earlier:`, and neither word is a kind, so a row cannot be confused with a
note.

## Wanted

One memory is one line, and the header reads without inference:

```
<memory>
rule | or board | now: the OR board is reviewed on Mondays
episode | release checklist | 2mo: the release checklist lives in docs/
fact | alice | 3d: alice owns the deploy pipeline
fact | - | now: the deploy pipeline has no owner
more: project 4 | memory_recall(subject) for the rest
</memory>
```

What the render must do:

- **The kind is always present**, `fact` included, so every line starts with a
  known token instead of sometimes starting with a subject.
- **Fields are separated by a bare `|`** with one space either side. The kind is
  the first field, the subject the second, the age the third, and the statement
  follows the colon. The colon ends the header; a `|` inside a statement is fine
  because it comes after the colon.
- **An empty subject renders as `-`**, so the field count never changes.
- **No bullet.** The newline already marks the row, and the block is a set of
  facts, not a list of options. A bullet costs about two characters on every row
  of every turn and marks nothing new.
- **Coverage lines and the rules notice are plain lines**, as they are now.
  `more:` and `earlier:` name something other than a memory.
- **A clipped row keeps its id at the end**, per the id issue, after the
  ellipsis: `fact | alice | 2mo: the rest of a long statement... m_xxx`.
- **The rules block renders `{content}` per line**, with no fields of its own and
  no bullet: a rule has no subject and no age, and the block is already the
  rules. The id goes there too, per the same issue.

Pricing, against the current store's shape:

- Two separators add four characters to a row, and showing `fact` adds five on a
  fact line. Ten rows cost roughly 15 to 20 tokens more per turn.
- The same change removes the id, which costs about 90 tokens over ten rows. The
  render is still about 70 tokens cheaper than today.
- The rules block loses its ids and gains nothing, so it is cheaper and shows
  more rules.
- The digest p95 gate (300 tokens) is unaffected; the measured p95 is 139.

## Why

The model reads the digest, the recall tool output, and the rules block as prose.
Unmarked fields work while the rows repeat a shape, but a multi-word subject is
the common case in the reference home, and the field that ends it is the one
field whose alphabet is not obvious. When the model guesses wrong, it loses the
subject or attaches the age to the statement, and nothing in the output shows the
misread.

A bullet would not fix that: the fields are the ambiguity, and the newline is
already the row boundary. A separators-only line is self-describing at about two
tokens a row, which is the smallest change that makes the header unambiguous. A
legend in the prompt is the alternative, and it costs a line on every turn even
when the digest is short.

## Scope

- Do not add bullets to the digest, `memory_recall` output, or the rules block.
- Do not change the statement text, the content limit, the subject limit, the
  age alphabet, or the coverage keywords.
- Do not change what is selected for the digest. This is a render change.
- Do not add a legend to the prompt. The punctuation is the explanation.
- Do not add a field that a memory does not have. The header is kind, subject,
  age only.
- Do not change the rules budget or its fill. The separator change does not
  touch the rules line.

## Test

- A digest of three memories renders three lines, each starting with a kind, each
  carrying ` | ` between kind, subject, and age, and a colon before the
  statement. No line starts with `- `.
- A fact renders with `fact`; a row with no subject renders `fact | - | now:
  ...`; a multi-word subject stays one field.
- A statement containing `|` and a statement containing `:` after the header
  still render as one row and read back correctly.
- `memory_recall` output uses the same header as the digest, so one format is
  tested once.
- The rules block renders one plain line per rule and a plain notice.
- A clipped legacy row ends with its id.
- The digest and rules token estimates are measured after the change; a ten-row
  digest stays well below the 300-token p95 gate, and the rules fill still stops
  at 800.
- The tests that count digest rows count lines that start with a known kind
  token, since `more:` and `earlier:` are the only other lines.

## Documents

- SPEC 7.4 (the digest format), 7.6 (the rules render), Appendix A, and the
  Draft bump.
- DECISIONS: a new entry for the delimiter, the mandatory kind, the `-` subject
  placeholder, the rejected bullet, and the cost trade against the id.
- The `memory_recall` schema description if it names the line format, and the
  `digest::line`, `digest::rules_body`, and `digest::recall_body` doc comments.

## Not established

- Whether `|` is the best delimiter. It is already the separator inside the
  coverage line, so the block has one delimiter convention; `·` would be harder
  to confuse with a statement but does not match the existing style.
- Whether a bullet would help a model that misreads the block. It was considered
  and left out: it adds no boundary, and no test can measure model comprehension
  without a live call, which the repository does not do.

## Resolution

`digest::line` renders `kind | subject | age: statement`. The kind is always
present from `Kind::code`, an empty subject is `-`, and there is no bullet. A
whole row carries no id; a clipped row appends its id after the ellipsis, and
`digest::tool_line` always appends it for `memory_recall`. The rules block renders
one plain `content` line per rule with no id. `digest::is_memory_line` is the row
predicate the row-count tests use. `clip` returns `Clipped { text, truncated }`,
so the id follows the same decision that cut the statement.

Commits: `94e1073` (test), `5f2a9ca` (feat), `01837f5` (docs).

Verified with the `digest::tests` header, dash subject, multi-word subject, pipe
and colon, clipped-id, tool-id, rules, and `is_memory_line` tests; the
`s6_agent` row count; `s6_prefix_stability`; and the memory benchmark. Ten full
rows measure 611 tokens and the worst case 1,071, both under the count bound; the
benchmark p95 is 97. `./scripts/ci.sh` passes.
