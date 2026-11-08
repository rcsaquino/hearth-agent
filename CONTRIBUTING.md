# Contributing

## The spec is binding

`SPEC.md` defines the behavior. `DECISIONS.md` records deliberate choices and deviations. A change that contradicts the spec needs a decision entry and a spec draft bump in the same pull request. Do not silently change a contract.

## Test-driven development

Work at the seams: the storage API, the provider wire format, the tool contract, the transport, and the CLI. For each behavior:

1. Add a failing test that states the expected behavior and run it. Confirm it fails for the expected reason.
2. Implement the smallest change that makes it pass.
3. Commit the test and the implementation separately: `test: ...` then `feat: ...` or `fix: ...`.

Do not write catch-up tests for code that already landed. If a test passes the first time, say so in the report rather than pretending it was red. Keep unit tests local; use fake servers for network behavior and never call a live service from a test.

## Design rules

- YAGNI: build what the current milestone needs.
- KISS: prefer the simplest structure that meets the contract.
- DRY: one owner for each rule.
- No async runtime and no ORM; the storage layer is a concrete SQLite implementation with one writer thread.
- Add a dependency only when it is used, and record the reason.

## Local gate

Run the full gate before every commit:

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/ci.sh
```

`scripts/ci.sh` is the authoritative gate. The worktree must be clean at the end.

## Commit style

- `test:` a failing test only.
- `feat:` the implementation that makes it pass.
- `fix:` a correction with the test that proves it.
- `style:` formatting only.
- `docs:` documentation only.

Keep each commit to one behavior. Do not combine a test and its implementation.

## No secrets

Never commit a token, key, password, connection string, or personal identifier. Use `${ENV_VAR}` references in settings and placeholders in documentation. Before publishing, run:

```bash
HEARTH_PUBLIC_DENYLIST=~/.hearth-public-denylist ./scripts/public-scan.sh
```

The deny list is private and outside the repository. The scan reports the file, not the value. A clean run prints `public scan: clean`.
