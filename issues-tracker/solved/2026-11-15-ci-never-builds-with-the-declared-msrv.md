# CI never builds with the compiler version the manifest promises

| | |
| --- | --- |
| Kind | chore |
| Reported | 2026-11-15 |
| Area | `.github`, `Cargo.toml` |

## Requested

CI should prove that the workspace builds with the minimum Rust version the
manifest declares, or the manifest should stop declaring one.

`Cargo.toml`:

```toml
rust-version = "1.89"
```

`.github/workflows/ci.yml` uses the current stable toolchain:

```yaml
      - uses: dtolnay/rust-toolchain@stable
```

No job pins 1.89. `AGENTS.md` says "Rust 1.89 or newer, edition 2024", and
`README.md` repeats it, so a user on 1.89 is told the tree works. A dependency
bump or a newly stabilized API can raise the real floor without any CI signal,
and the first evidence is a user's build failure.

The code already relies on at least one recent stabilization
(`File::try_lock` at `crates/hearth-runtime/src/daemon.rs:73` and
`crates/hearth-runtime/src/doctor.rs:121`, stabilized in 1.89), so the declared
floor is not arbitrary and is worth keeping honest.

## Why it is worth doing

The MSRV is a promise to the person building from source. A gate that compiles
with the oldest supported toolchain turns a late user report into a red job at
the commit that broke it. Without it, the value in `Cargo.toml` is unverified
documentation.

## Verification

Install 1.89 and run `cargo build --workspace --all-targets`. Then run
`./scripts/ci.sh` on 1.89. Comparing that result with stable shows whether the
manifest is true today; adding the same build as a CI job keeps it true.

## Resolution

Added an `msrv` job that checks out the tree, installs `dtolnay/rust-toolchain@1.89`,
and runs `cargo build --workspace --all-targets`. `scripts/test-scripts.sh`
reads `rust-version` from the workspace manifest and fails when `ci.yml` does
not name that toolchain.

Commits: `3ac2ddd` (test), `3235beb` (fix). Verified by `./scripts/test-scripts.sh`.
