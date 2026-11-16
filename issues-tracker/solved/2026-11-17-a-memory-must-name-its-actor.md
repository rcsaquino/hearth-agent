# A memory must name its actor

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-17 |
| Area | `hearth-memory` |

## Observed

A memory is injected with no conversation around it. `build_prompt` in
`crates/hearth-core/src/prompt.rs` places the digest as a system block directly
before the newest user message, and `digest::line` renders each row as
`<id> <kind> <subject> <age>: <content>`. The model reading that block has the
previous turns, but the block itself carries no antecedent. The block is also
read again after a compaction, when the turns that explained it are gone.

So a statement has to stand on its own. Several write paths store statements
that do not:

- `store::validate` in `crates/hearth-memory/src/store.rs` checks trimming, the
  content limit, the subject limit, credential shape, and the kind/pin rule. It
  does not check that the statement names its actor.
- The `memory_save` description in `crates/hearth-memory/src/tools.rs` asks for
  "One statement, at most 150 characters, in the user's terms", and the `subject`
  field is described as a findability aid. Storing the user's words verbatim
  keeps "I" and "you" inside the content, and the subject is optional.
- The deterministic extractor keeps the sentence as typed. `from_remember`
  stores everything after the marker unchanged; `from_preference` stores
  `I prefer dark mode` and sets the subject to `user`; `from_possessive` stores
  `my sister is Alice`. `leading_subject` refuses to treat `i`, `my`, `we`,
  `our`, `the`, `a`, and `an` as a name, but `he`, `she`, `they`, and `it` are
  not names either, so the subject comes out empty.
- `SUMMARY_INSTRUCTION` in `crates/hearth-runtime/src/compaction.rs` asks the
  model for "one statement" and a subject, not for a standalone sentence. The
  same request reaches the store from `MemoryService::write` and from the
  compaction save.

Two shapes come out of this. `Remember that he likes apples.` becomes
`he likes apples` with an empty subject, and `I prefer dark mode.` becomes
`I prefer dark mode` with subject `user`, so the rendered line reads
`m_... pref user now: I prefer dark mode`.

The vague statement also retrieves worse. `rank::coverage` in
`crates/hearth-memory/src/rank.rs` matches query terms against
`subject + content`, and the subject channel in `store::by_subject` needs a
non-empty subject, so a row that says `he` and carries no subject has almost
nothing to match.

Existing rows are unaffected by any rule added now; the digest cannot repair
them at render time.

## Wanted

Every stored statement must read correctly with no conversation around it: the
actor is named in the content, not left as a pronoun.

The rule has three parts:

- **Third person** (`he`, `she`, `they`, `them`, `their`, `it`) must be replaced
  by the name from the message. If the write has no antecedent, refuse it. Do
  not guess.
- **First person** (`I`, `me`, `my`, `we`, `our`) is the user. With no user name
  in `identity` today, the actor is `the user`, in the content and in the subject
  alike, so `I prefer dark mode` becomes `the user prefers dark mode`.
- **Second person** (`you`, `your`) is the agent, named by
  `identity.agent_name`, or the user when the statement is about the user. The
  write path must supply that name; `MemoryService` does not hold the identity.

The subject stays a normaliser for matching. It is never the only place the
actor appears: `subject = alice` with content `she manages the OR board` is the
shape to refuse, and so is an empty subject with a pronoun in the content.

Where it applies:

- **The tool refuses and names the fault.** `memory_save` is a model call, so
  the model can retry with a name. The error names the pronoun and the rule, and
  saves nothing, the way a length refusal does.
- **The extractor rewrites what it can and drops what it cannot.** It sees the
  user's message, so first person has a determinate meaning; third person only
  when the message names someone. A candidate whose antecedent it cannot find is
  skipped, not stored.
- **The instructions say the same thing.** The `memory_save` schema, the
  `SUMMARY_INSTRUCTION` in compaction, and any extraction prompt ask for a
  standalone statement and name the actor. The store remains the final gate for
  every path, including the legacy import and direct service writes.
- **Rules keep their shape.** A rule is an instruction, not a sentence about a
  person: `Never deploy on Friday` is standalone without an actor, and a leading
  imperative is not a pronoun fault.

Existing rows are not rewritten or deleted. A vague row can be replaced with
`supersedes` or removed with `memory_forget`, and the next write of that fact is
held to the new rule.

## Why

The digest is auto-injected on every turn, so an unresolved pronoun is paid for
again and again, and a wrong guess about who `he` is worse than a missing row:
it attaches a fact to the wrong person and the model has no way to tell.

The current instructions pull against each other. The tool asks for "the user's
terms", which preserves `I`, while the digest renders the statement as a neutral
system line where `I` reads as the assistant. One rule at the store removes the
conflict and matches how the content limit is already enforced.

It also makes retrieval work. A named actor gives `coverage` and the subject
channel something to match, and the embedding of a named statement is closer to
the question that will recall it than an embedding of `he likes apples`.

## Scope

- Do not add a model call to rewrite memories at write time. A rewrite costs
  tokens, adds latency to a save, and can invent detail. The deterministic paths
  resolve what they can and refuse the rest.
- Do not rewrite or delete existing rows. The change is a write rule.
- Do not require every statement to start with a subject. A rule or an
  imperative has no grammatical subject and is still standalone.
- Do not add a user-name setting here. `identity` has `agent_name` only; a user
  name is a separate change, and `the user` is correct until it exists.
- Do not change retrieval scoring, the content limit, or the subject limit.

## Test

- `memory_save` with `He likes apples` is refused with `invalid_input`, names the
  pronoun, and writes nothing; `Bob likes apples` and `the user likes apples` are
  accepted.
- `memory_save` with `I prefer dark mode` is refused, and the retried
  `the user prefers dark mode` is accepted and renders as a standalone line.
- A statement with `subject = alice` and content `she manages the OR board` is
  refused; `alice manages the OR board` is accepted with the same subject.
- The extractor turns `I prefer dark mode.` into `the user prefers dark mode`
  with subject `user`, and drops `Remember that he likes apples.` when the
  message names no antecedent. `Remember that Alice likes apples.` keeps the
  name.
- A rule such as `Never deploy on Friday` is accepted without an actor.
- The `memory_save` schema text and `SUMMARY_INSTRUCTION` both contain the
  standalone requirement, and the store check is exercised independently of
  either string.
- An existing vague row still renders; the new rule does not hide it.

## Documents

- SPEC 7.2 (what a memory is), 7.3 (write rules), 7.4 (recall), and the Draft
  bump.
- DECISIONS: a new entry for the standalone rule, the actor mapping for first,
  second, and third person, and why the check is deterministic.
- The `memory_save` schema text and its test in `crates/hearth-memory/src/tools.rs`,
  the extractor tests in `crates/hearth-memory/src/extract.rs`, and the
  compaction instruction text in `crates/hearth-runtime/src/compaction.rs`.

## Not established

- Whether a statement about the user should say `the user` or a configured name.
  `identity` has no user name today; adding one changes the prompt, the settings
  form, and every existing statement that says `the user`.
- Whether an extractor candidate with a third-person pronoun can ever be resolved
  from the same message without a model. The message may name a person earlier,
  but linking the pronoun to that name is parsing, not a lookup the extractor
  currently does.
- Whether a leading demonstrative (`This is the release checklist`) should be
  refused like a pronoun. It is the same fault in a different word, and the
  parser can spot it, but no write in the repository produces that shape yet.

## Resolution

`actor::pronoun` finds the first first, second, or third person pronoun on
non-alphanumeric boundaries, and `store::validate` refuses a content that carries
one with `invalid_input` and an error that names the pronoun and the replacement.
The check is deterministic and calls no model. `extract` rewrites first person
(`my` to `the user's`, a leading `I` or `we` with the verb agreed to the singular)
and drops a candidate whose remaining pronoun it cannot resolve; `acceptable`
rechecks every candidate. The `memory_save` content description and
`SUMMARY_INSTRUCTION` ask for a standalone statement with a named actor, and the
four memory schemas measure 799 tokens. A rule keeps its imperative shape; an
existing vague row is not hidden or rewritten.

Commits: `8b252af` (test), `2b5bce8` (feat), `11bd310` (docs).

Verified with `actor::tests`, the store tests for a pronoun refusal, a named
actor, and an actless rule, the extractor tests for first-person rewrites, a
dropped third person, and a kept name, the tool tests for a pronoun refusal and a
retried first, second, or third person statement, the schema and compaction
instruction tests, and the existing-vague-row render test. `./scripts/ci.sh` and
the memory benchmark pass.
