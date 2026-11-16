# The agent loop has no turn cap

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-16 |
| Area | hearth-runtime |
| Build | 1d380d9 |

## Observed

A generation task answers on turns that carry no user message. It does not fail,
and it does not stop. It keeps requesting.

Task `01a1e01b-8518`, in conversation `01a1dbae-7d88`, started at 00:47:42 from
the one-word message "Sure". It settled as `done` at 00:50:52 at turn 19. The
conversation gained exactly one user entry for that task, at seq 2702. Every turn
after the first rebuilt the prompt from a history whose last user message was
still "Sure".

The same shape recurs on every task in this session. `01a1e018-f657` reached
turn 11 from "Okay go". `01a1e01f-2f28` reached turn 13 from "Go".

## Mechanism

`crates/hearth-runtime/src/agent.rs:461` opens `'turn: loop {`. The only
increments are at `agent.rs:438` and `agent.rs:667`, both guarded by
`if !calls.is_empty()`. The loop exits when the model returns text with no tool
calls, when the context gate at `agent.rs:535` fails, or when a cancel token is
set.

There is no turn limit, and its absence is deliberate. `agent.max_turns` no
longer exists in settings: `crates/hearth-config/tests/budget_settings.rs:48` is
named `agent_has_no_turn_limit_setting`, and
`crates/hearth-runtime/tests/s6_agent.rs:565` is named
`a_long_tool_loop_is_not_cut_short_by_a_turn_limit`, which scripts 125 tool
rounds and requires them all to run.

## Closure

**Working as intended.**

The absent cap is a decision, not an oversight. Two tests encode it:

- `crates/hearth-runtime/tests/s6_agent.rs:565`,
  `a_long_tool_loop_is_not_cut_short_by_a_turn_limit`, runs 125 tool rounds and
  requires all of them to complete.
- `crates/hearth-config/tests/budget_settings.rs:48`,
  `agent_has_no_turn_limit_setting`, pins that no such setting exists.

Rey confirmed on 2026-11-16 that an unbounded tool loop is the intended design.
The retired `agent.max_turns` setting stays retired.

So the turn count is not a fault. The observation stands — 19 turns from one
word — but the cause is a loop doing the work it was asked to do, and the fix
belongs elsewhere.

## Remainder

One part of the original report is still true and is split out.

`docs/DECISIONS.md:342` states: "The agent turn limit counts provider requests.
Reaching it saves a notice, settles the submission unanswered with reason
`turn_limit`, and fails the task." That describes behaviour the code does not
have and that Rey does not want. The documentation contradicts the decision, and
a reader who trusts it would look for a limit that is not there.

Filed as `open/2026-11-16-the-decision-log-still-documents-a-turn-limit.md`.

The turns themselves remain answerable as though a person had spoken, which is
the separate fault in `open/2026-11-16-the-runtime-note-reads-as-a-request.md`.
That fault does not depend on a cap and is not closed by this one.
