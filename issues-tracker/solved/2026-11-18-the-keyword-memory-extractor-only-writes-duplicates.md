| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-18 |
| Area | `memory` |

## What happens now

After every reply, a keyword pass reads my message and writes anything that looks
like a durable statement to memory. It recognises a short list of openings:
"remember that", "from now on", "I prefer", "my X is Y", "X is my sister", and a
few like them. It rewrites what follows into third person and stores it.

It has written three statements in this store since the harness went live. All
three were facts I had already saved myself, in my own words, minutes earlier:

- Kya's nut allergy. I had written it. The keyword pass wrote a reworded copy.
- Moving from the previous harness to this one. I had written it. Reworded copy.
- The workout programme I follow. I had written it. The copy dropped detail the
  original had, including that the programme is weighted and runs about three
  times a week.

So it has not caught anything I missed. Every statement it produced was a second
copy of an existing one.

That matters more than it sounds, because a reworded copy is the one kind of
duplicate the store cannot stop. Identical wording is caught. Rewording is not,
and rewording is exactly what this pass does, so it manufactures the duplicate
class that escapes the guard.

## What should happen

Either remove the keyword pass, or stop it writing memory directly.

If it is kept, the useful shape is a check rather than a writer: notice a message
that asks for something to be remembered, see whether the turn saved anything,
and if it did not, give the model one nudge to save it properly. The model then
writes the statement with the right subject, the right kind, and the context
around it, instead of a string-rewriting pass turning "verify" into "verified" and
guessing a subject from capital letters.

The promise being protected is "remember this" being kept. That promise is kept
by the model saving it, not by a second writer with worse judgement.

## Why it is worth deciding

Two reasons. The obvious one is that it only ever adds duplicates today, and each
one costs a recalled line forever.

The less obvious one is that it makes the memory store look maintained when it is
not. It is cheap and always runs, so it reads like a safety net, and the real
failure, which is me not saving something I should have, stays invisible because
the pass quietly writes an inferior copy of whatever it recognised.

## What is not established

How often the model actually fails to save something it was asked to save. If
that happens regularly with modern models, the nudge version is worth having. If
it is rare, removing the pass entirely is simpler and costs nothing.

The pass costs no tokens and no latency, so the argument against it is about
duplicate noise and misleading coverage, not about runtime.

## Resolution

Removed. The keyword pass no longer exists, and the agent is the only writer of
memory.

- `hearth-memory/src/extract.rs`, `hearth-runtime/src/extraction.rs`, its tests,
  and the daemon's post-turn capture thread are gone: 835 lines deleted, 8 added,
  across 12 files. `RunOutcome.user_entry_ids` and `suggest_subject` existed only
  to feed that thread and are gone with it. `Source::Extract` stays, because the
  compaction summary still writes statements it read from the range it summarised.
- The promise moved to where the judgement lives. The `memory` tool description
  now ends: "Save when the user asks you to remember something." The `save`
  action is the same one the model already used, so nothing new is reachable and
  nothing new costs tokens.
- Why removal, not the nudge: the nudge needs durable pending state between the
  answer and the next turn, and the model already reads the request and the store
  in the same turn. The issue says a nudge is worth having only if the model
  regularly fails to save. Nothing in this home measured that, and the pass had a
  confirmed defect: the pair it wrote scores 0.476 Jaccard and 0.786 containment,
  so the store's own `same_statement` refuses it and both rows stay current. A
  safety net that manufactures the one duplicate class the store cannot catch is
  worse than no net.
- Commit `d5b9143`, docs in `66759ce`.

Verified: `./scripts/ci.sh`, `./scripts/acceptance.sh`, and a real store driven
through the `memory` tool. Two saves of a related pair now answer
`related: m_c9mxpf6qt9 (0.57 overlap) ... If the two say the same thing, forget
one.` instead of calling them separate. Nothing writes to memory after a run.
