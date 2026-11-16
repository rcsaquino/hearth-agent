# Spill belongs in the temp folder

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-17 |
| Area | `hearth-runtime` |

## Observed

`Agent::spill_output` in `crates/hearth-runtime/src/agent.rs` writes any tool
result above 4,000 estimated tokens to `AgentConfig.spill_dir`, redacted and at
mode 0600, and hands the model a `spill/<name>.txt` reference in place of the
full text. The daemon sets that root to `home.join("spill")` in
`crates/hearth-runtime/src/daemon.rs`. `Workspace` in
`crates/hearth-tools/src/workspace.rs` resolves `spill/` reads to the same root,
so `files`, `documents`, `media`, and helper output all read through it.

Nothing deletes a spill file. `Maintenance` in
`crates/hearth-runtime/src/maintenance.rs` prunes prompt blobs, helper retention
prunes task rows, and `Log::rotate` bounds the log, but no pass looks at the
spill directory. Spill is the only directory in the home with no retention rule,
so it accumulates one file per oversized tool result for the life of the home.
Each file is bounded by the producing tool (fetch caps at 2MB, shell output is
captured under `CAPTURE_CAP`), but the directory is not.

The home layout is fixed: `HOME_DIRECTORIES` in `crates/hearth-runtime/src/home.rs`
creates `spill` from both `init` and `restore_home`, and `LAYOUT` in
`crates/hearth-runtime/tests/home_recovery.rs` asserts it. SPEC 3.1's home table
lists it as recreated too.

The files outlive the turn that wrote them. `spill_output` embeds the path in
the saved tool-result block, so a reloaded conversation still points at
`spill/<name>.txt`, and the model is told to read it when it needs the rest.

SPEC 3.1 already declares this data temporary. It says temporary work goes to
`$TMPDIR/hearth-*`, may vanish at any time, and that nothing durable belongs
there. Spill is exactly that: a redacted copy of one tool result whose only
consumer is an optional read by the model. Nothing in the repository reads
`TMPDIR` today, and no code puts scratch data outside the home.

## Wanted

Spill lives in the temp folder: the root is `/tmp/hearth/spill`, with `TMPDIR`
honoured when it holds a path, matching the `$TMPDIR/hearth-*` rule the SPEC
already states.

What follows from that:

- **The home stops carrying it.** Remove `spill` from `HOME_DIRECTORIES` in
  `crates/hearth-runtime/src/home.rs`, from `LAYOUT` in
  `crates/hearth-runtime/tests/home_recovery.rs`, and from SPEC 3.1's home
  table. `init` and `restore_home` no longer create it. The temp rule in 3.1 is
  the one place that describes it.
- **The daemon creates it on demand.** `spill_output` already calls
  `create_dir_all` before a write; that stays the only creation path. Nothing
  assumes the root exists at startup.
- **Loss is normal.** A spill may disappear at any time: reboot, tmpfs, a
  cleaner, or a private `/tmp` if a unit ever provides one. A read of
  `spill/<name>.txt` after that is an ordinary not-found for that file. It must
  not become a root error, and it must not be retried as a fault.
- **Confinement does not move.** Writes still refuse the `spill` prefix, reads
  still resolve only through the spill root, and `/tmp` does not become a
  general read root. The root is passed in, not derived from the path.
- **The model-facing text does not change.** It still says
  `spill/<name>.txt`; the keyword names the root, not the home.

## Constraints

`/tmp` is world-writable and `/tmp/hearth` is a predictable name, so the fixed
path is acceptable only if creation is deliberate. The required items are
defects if they are missing. The accepted items are consequences to record, not
problems to solve.

### Required

- **Create the root private.** Mode 0700, set at creation. Do not inherit
  `/tmp`'s mode, and do not rely on the 0600 file mode to protect a directory
  entry it does not cover.
- **Refuse an unsafe root.** If the path already exists and is a symlink, is not
  a directory, or is not owned by the current user, refuse it and report the
  failure the way a failed directory is reported today. `create_dir_all`
  succeeds on a directory someone else made, and the owner of that directory can
  replace a spill file with one the model will read as tool output.
- **Treat a missing root as empty, not as an error.** `Workspace::resolve_readable`
  canonicalizes the selected root and returns `missing_root` ("the root does not
  exist") when it is absent, and `Workspace::read_roots` does the same for
  absolute paths. A wiped `/tmp` must degrade a read to not-found, not break
  every `files` call.
- **Treat an empty `TMPDIR` as unset.** `std::env::temp_dir()` returns it
  verbatim rather than falling back to `/tmp`, so a naive honouring writes a
  relative `spill/` into the daemon's working directory. The root resolution is
  a function of the environment value, and it must reject an empty one.
- **Do not make task recovery depend on a surviving spill.** Check whether a
  requeued or resumed task can re-read a spill before relying on the root.

### Accepted

- **One root is shared by every home.** `~/.hearth`, a `.devhome`, and a
  `TempDir` home in a test all resolve to the same path, and spill names are
  call ids. Two live daemons can collide, and `files list spill/` lets one
  home's model enumerate another's output. This is accepted for the single-home
  product model: record it in DECISIONS. Tests point the root at their own
  `TempDir` and never at the constant.
- **The root may be RAM.** On the reference host `/tmp` is tmpfs (3.9G,
  `usrquota`), and `systemd-tmpfiles` clears entries older than 10 days. A
  spill file is bounded by its producer: 2MB for a fetch, `CAPTURE_CAP` for a
  shell command, and a result only spills above 4,000 estimated tokens. Normal
  use therefore holds tens to hundreds of MB over the window, not gigabytes;
  roughly 200 large fetches a day would reach the cap. The cost is real but
  bounded, and it shares `/tmp` with every other program.
- **`PrivateTmp` is not a present risk.** `packaging/hearth.service` is a user
  unit, and the `systemd.exec` manual says user services cannot use most file
  system namespaces without `PrivateUsers=true`. If a future unit enables one,
  the daemon gets a private `/tmp`: the root is empty after every start and
  invisible to the TUI. That is acceptable for scratch data. Record it in
  DECISIONS as a consequence, not a surprise.

### Preference

- Put one segment between the fixed name and `spill` when the home is not the
  default one, for example `/tmp/hearth/<home>/spill`, so a development home
  cannot read the live home's output. This is optional: the fixed path is
  accepted above. One authoritative definition of the name is enough; the fix
  does not have to add a setting for it.

## Why

Spill is tool output the user never asked to keep: file contents, search
results, fetch bodies, command output, redacted but still personal. Keeping it
forever in the home, which is copied by hand, moved between machines, and
scanned for secrets, is the wrong default for data whose only purpose is a
single optional read. The home grows without bound and nothing reports by how
much. The built-in backup copies only `hearth.db`, so the cost is the home
tree, not a snapshot.

The repository's own rule already places this data in temporary space. The gap
is that the rule was written and the code kept the durable shape.

## Not established

- Whether task recovery ever re-reads a spill written before a restart. If it
  does, a per-boot root would lose output recovery needs, and this change needs
  a recovery-side answer.
- Whether `TMPDIR` is ever set differently from `/tmp` on a host Hearth runs on.
  The SPEC rule names `$TMPDIR`; the request names `/tmp`. On the reference host
  it is unset in both the shell and the systemd user manager environment.
- Whether any operator looks in the spill directory by hand. No CLI or TUI
  surface reads it today, so no documented workflow would break.

## Test

- An oversized tool result writes exactly one file under the temp root; a result
  below the inline cap writes none and creates no directory.
- The root is created 0700. A symlink at the root, a root that is a regular
  file, and a root owned by another user are refused rather than used. The
  ownership case needs a second uid: keep it out of the portable suite, or skip
  it when the uid cannot change.
- The root resolution treats an empty `TMPDIR` as unset. Test the resolution
  with an empty value; do not write to `/tmp` to prove it.
- `init` and `restore_home` do not create `spill` under the home, and `LAYOUT`
  no longer lists it.
- Deleting the root mid-run makes `files read spill/<old>.txt` return a
  not-found for that file, not `missing_root`, and the next spill write
  recreates the root so a fresh reference reads back.
- A write through the tool to `spill/...` is still refused with `invalid_path`.
- No test writes to `/tmp/hearth` itself; each test points the root at its own
  `TempDir`.

## Resolution

The spill root is now `$TMPDIR/hearth/spill`, or `/tmp/hearth/spill` when `TMPDIR`
is unset, empty, or relative. `spill::root` resolves it and `spill::ensure_root`
creates the root and its `hearth` parent at mode `0700`, refuses a symlink, a path
that is not a directory, and a directory owned by another uid, and is called by
`Agent::spill_output` and the helper report write. `home::HOME_DIRECTORIES` and
the `init` and `restore_home` layouts no longer include `spill`;
`workspace::Workspace` treats a missing spill root as a not-found file and
`read_roots` skips a missing spill root, so a wiped `/tmp` does not break an
absolute workspace read. A write through the tool to `spill/...` is still refused
by `Workspace::resolve_write`.

Commits: `ff652a4` (test), `f316ed4` (feat), `ad3c468` (docs), `a31adfb` (the
write-refusal regression test).

Verified with `spill::tests` (resolution, empty and relative `TMPDIR`, `0700`
creation, symlink and owner refusal), `hearth-tools` `a_missing_spill_root_*` and
`a_write_through_the_tool_to_spill_is_refused`, `s6_agent`
`a_large_file_read_spills_and_can_be_read_back` (root `0700`, file `0600`, no
home spill), `home_recovery`, `init_home`, and `daemon_lifecycle`. No test writes
to `/tmp/hearth`. `./scripts/ci.sh` passes.
