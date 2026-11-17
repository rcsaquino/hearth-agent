# Media pruning is documented but has no implementation

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-17 |
| Area | `hearth-runtime` |

## Observed

`docs/SPEC.md` 11.3 says media can be pruned by age and that missing references
are reported. No code scans `media/inbound` or `media/outbound`. `InboundMedia`
creates one directory per UTC day and never removes anything. The only pruning
paths in the runtime are `BackupHandler` for prompt blobs, `backup::prune` for
snapshots, `MemoryService::prune_vectors`, and `Session::prune_helpers`.

Attachments and generated files therefore grow without bound. A home that
receives photos or voice notes for years keeps every one, and every backup of
the home copies them unless the operator removes them by hand.

## Wanted

Either implement the documented age-based prune with a report of references that
would break, or remove the promise from SPEC and state that media is operator
managed. The choice belongs in the spec and the settings surface, not only in a
comment.

## Verification

Search the runtime for a media directory walk or an age check on a media path.
There is none. The `media` root is created by `home::scaffold` and written by
`InboundMedia::save` and the outbound writers, and no code deletes from it.

## Resolution

Removed the media-pruning promise. `docs/SPEC.md` 11.3 now says media is
operator managed and Hearth never deletes a file under `media/`; an operator who
wants a smaller backup removes old inbound and outbound files by hand. The
decision log records why the alternative, an age-based delete with a
broken-reference report, is not built.

Commit: `e8c20f1`. This is a docs-only change; `scripts/check-docs.sh` passes.
