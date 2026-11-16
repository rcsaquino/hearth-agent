# The acceptance script names a test target that no longer exists

| | |
| --- | --- |
| Reported | 2026-11-15 |
| Build | 330ccfb |
| Area | `scripts` |

## Observed

`./scripts/acceptance.sh` stops partway through and exits 101.

```
== prompt cache and overhead ==
error: no test target named `tokens_bench` in `hearth-eval` package
help: available test targets:
    memory_bench
    memory_reachability
    s7_crash
```

Because the script runs under `set -euo pipefail`, nothing after that line runs:
the soak, the secret scan, and the closing `acceptance ok` line. The documented
gate therefore cannot report success on a clean tree.

## Mechanism

`scripts/acceptance.sh` line 33:

```bash
echo "== prompt cache and overhead =="
cargo test -p hearth-eval --test tokens_bench
```

`crates/hearth-eval/tests/` holds `memory_bench.rs`, `memory_reachability.rs`, and
`s7_crash.rs`. There is no `tokens_bench` target, and `crates/hearth-eval/Cargo.toml`
declares no `[[test]]` entry for one.

`crates/hearth-eval/tests/tokens_bench.rs` was deleted in `9681d62` ("feat: rebuild
memory and refine the terminal operator surface"). The script was last changed in
`b49ba79`, before that deletion, so the two drifted apart.

`AGENTS.md` still lists `./scripts/acceptance.sh` under "Additional checks" and
describes it as crash, scheduler, fake Telegram, benchmarks, and soak.

## Consequence

Two failures follow. Following the documented command, an operator never reaches
the soak or the secret scan, so a regression in either is invisible to the gate
that is supposed to cover it. That is the state of the tree now: the acceptance
gate is red for a reason unrelated to any behaviour under test, which also makes a
genuine red indistinguishable from this one.

## Not established

Whether the prompt-cache and overhead coverage that `tokens_bench` provided moved
into another target or was dropped. `memory_bench` measures recall, tokens, and
retrieval latency, which is not the same claim. Deleting the line would make the
script run again; whether the coverage should move too is the open question.

## Verification of a fix

Run `./scripts/acceptance.sh` from a clean tree and show it reaching the closing
line. If prompt-cache coverage is meant to stay, name the target that now carries
it, and state what, if anything, still checks the cached-prefix overhead.

## Resolution

`scripts/acceptance.sh` now runs `cargo test -p hearth-runtime --test s6_prefix_stability`
and the fixed-overhead test in `s1_prompt` under the "prompt cache and overhead"
heading, the targets that carry that coverage today. `scripts/test-scripts.sh`
now parses every `cargo test -p PKG --test TARGET` in `scripts/` and fails when
`crates/PKG/tests/TARGET.rs` is absent, so the drift cannot recur silently.

Commits: `bf2e3d7` (test), `96a1e17` (fix). Verified by `./scripts/test-scripts.sh`
and a full `./scripts/acceptance.sh` run reaching its closing `acceptance ok`
line.
