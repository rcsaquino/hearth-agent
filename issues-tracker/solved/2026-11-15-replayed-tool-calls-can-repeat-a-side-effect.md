# A replayed tool call can repeat a side effect

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-runtime`, `hearth-telegram` |

## Observed

If the process dies after an assistant entry with tool calls is saved and before
the matching tool result is saved, the next run replays those calls. Replay
executes the same tools inline, with no effect row, no idempotency key, and no
guard for tools that act outside the database.

`crates/hearth-runtime/src/agent.rs:435`:

```rust
let pending = pending_calls(&history);
if !pending.is_empty() {
    self.execute_calls(&submission, &claimed.id, &pending, events)?;
```

`pending_calls` (`agent.rs:984`) collects every tool-call block that has no
matching tool-result block in the history. `execute_calls` (`agent.rs:770`)
dispatches each one through the same inline path used for a fresh call:

```rust
let outcome = self.config
    .tools
    .call(&call.name, &call.arguments)
    .or(selected);
```

Two of those tools are not idempotent. The `shell` tool runs the command
(`crates/hearth-tools/src/shell_tool.rs`), and the `message` tool sends media
directly through `TelegramSender` (`crates/hearth-telegram/src/message_tool.rs:119`),
without an effect row. The replay itself is deliberate and recorded in
`docs/DECISIONS.md:339` ("Pending calls with no result are replayed before the
next provider request"), but that entry does not exempt unsafe tools.

## Consequence

A crash at the wrong point runs a shell command twice or sends the same album
twice. SPEC 4.6 requires an unsafe action to write its intent, act, then write
the outcome, and 4.5 requires each step to be idempotent from its checkpoint.
The replay has neither a checkpoint nor an effect for the tool call.

## Verification

Seed a conversation with an assistant entry that carries a `shell` call and no
tool result, then run one submission. Count the command's side effects. Run the
same setup through a crash driver that dies between the assistant save and the
tool-result save, then run a second submission. The command runs once per
replay, so the second sequence runs it twice.

## Resolution

Replay of a pending call is now classified. `shell`, `message`, `schedule`,
`browser`, and `media` act outside the database with no effect row, so a replayed
call is answered with a `not_replayed` tool error saying the call was not re-run
and can be re-issued. Read-only tools still replay. The two `execute_calls` call
sites pass whether they are replaying.

Commit: `1f4839c`. Verified by `a_replayed_shell_call_does_not_run_again`, which
seeds a pending `touch replay-marker` call, runs a submission, and asserts the
marker does not exist and the tool result says "not re-run".
