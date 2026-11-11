# Contributing

Hearth is a local-first Rust project. The product definition is
[SPEC.md](docs/SPEC.md). Implementation choices and deviations are recorded in
[DECISIONS.md](docs/DECISIONS.md). Coding agents should also read
[AGENTS.md](AGENTS.md), which has the build, test, architecture, and safety rules
in a compact form.

## Build and test

```bash
cargo build
cargo test --workspace
./scripts/ci.sh
```

`scripts/ci.sh` runs formatting, clippy with `-D warnings`, and the workspace
test suite. Run it before every commit. The optional release check is
`./scripts/test-scripts.sh`; it exercises the scan and packaging scripts and
builds a release tarball.

## Test-first

Every behavior begins as a failing test at a public seam:

- S1 core session and save path
- S2 storage contract
- S3 provider adapter
- S4 memory and skills
- S5 Telegram
- S6 agent loop
- S7 crash recovery and scheduler
- S8 operator surface

Do not batch tests before an implementation. Do not add production behavior
without a test that demanded it. Integration tests use real SQLite and only mock
system boundaries such as HTTP, the clock, and randomness. Keep expectations
independent of the implementation. A bug fix starts with a reproduction.

## Scope and style

Apply YAGNI, KISS, and DRY. Build what the current milestone needs, choose the
smallest direct design, and keep one authoritative definition for each rule.
Prefer a little repetition over a premature abstraction. New dependencies need a
one-line justification in `docs/DECISIONS.md`.

Use `cargo fmt` and address clippy rather than suppressing it. Keep modules small
and public interfaces narrow. The main seams are `Harness`, `Session`, `Storage`,
`Provider`, `Tool`, and the transport layers.

## Secrets and private data

- Never commit a token, key, password, private path, real name, Telegram id, or
  personal database.
- Use the placeholders `USER_NAME`, `123456789`, `~/.hearth`, and `example.com`
  in examples and docs.
- Keep personal development data in the gitignored `.devhome/` or a temporary
  directory.
- `scripts/secret-scan.sh` checks the tree and history for credential patterns
  and for a private deny-list kept outside the tree. A public release also
  requires a clean history; that rewrite is a separate decision.

## Local-only workflow

The repository is local. Do not push to a remote or open a pull request unless
the maintainer explicitly authorizes it. The prepared CI workflow is not a
publication step.

## Commits

Use short, imperative commits such as `feat: add the watch socket`,
`fix: bound the retry delay`, `test: require a stale lease to be requeued`, or
`docs: record the storage decision`. Keep tests with the behavior they cover.
Do not commit generated databases, media, logs, `target/`, or `dist/`.

## License

By contributing, you agree that your contribution is licensed under the MIT
license in [LICENSE](LICENSE).
