# Replay runs `task` and `skill` again after a crash

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-runtime` |

## Observed

When the process dies after an assistant entry with tool calls is saved and
before its tool result is saved, `Agent::run` calls `execute_calls` with
`replay = true` for the calls returned by `pending_calls`. `Agent::replay_safe`
allows every tool except `shell`, `message`, `schedule`, `browser`, and `media`.

Two allowed tools have side effects outside the database.

- `TaskTool`, whose spec name is `task`, creates a helper conversation and a
  helper task. Replaying it creates a second helper.
- `SkillTool`, whose spec name is `skill`, writes a learned skill file and
  appends to `CHANGELOG.md`, and a `save` increments the version. Replaying it
  writes a second revision for one model request.

`memory_save` with `supersedes` is also not obviously idempotent, because the
second run sees the row already closed and returns an error rather than the first
outcome.

The solved `replayed-tool-calls-can-repeat-a-side-effect` issue added the
classification for five tools. These names were not included.

## Verification

Seed an assistant entry with a `task` call and no tool result, then run one
submission with the crash driver. The helper count increases by one per replay.
Run the same shape with a `skill` save and count the versions under
`skills/learned/.history/`.

## Resolution

`replay_safe` now refuses `task`, `skill`, and `memory_save` alongside the five
tools it already refused. The model receives the existing `not_replayed`
result and re-issues the call only if it is still needed.

Commits: `17be087` (test), `a21bf16` (fix). Verified by
`a_replayed_task_skill_or_save_is_not_run_again`, which seeds one pending call
per name and requires each tool result to say the call was not re-run.
