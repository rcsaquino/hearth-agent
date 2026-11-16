# The settings example in the spec is not valid JSON

| | |
| --- | --- |
| Kind | chore |
| Reported | 2026-11-15 |
| Area | `docs` |

## Requested

The complete `settings.json` example in SPEC 10.1 should parse. It currently
contains a trailing comma after the last field of the `models` object.

`docs/SPEC.md:1475`:

```json
    "max_output_tokens": 32000,
    "session_header": "x-opencode-session",
  },
```

A trailing comma before `}` is not valid JSON. The example is presented as "a
complete install with every optional block enabled", so a reader is invited to
copy it into `~/.hearth/settings.json`. The copy fails at parse time with
`settings.json is not valid JSON: trailing comma`.

## Why it is worth doing

The binding product definition ships a literal file that the product refuses.
The surrounding text treats it as a working baseline, and a user following it
has no way to tell that the error is in the document rather than their edit.

## Verification

Extract the fenced block and parse it with
`serde_json::from_str`. It fails at line 1475. Removing the comma makes it parse;
`hearth_config::load` then accepts the document once the environment variables
are set.

## Resolution

Removed the trailing comma after `models.session_header` in SPEC 10.1. The
fenced block now parses with `serde_json::from_str` and carries the six
top-level blocks `identity`, `telegram`, `models`, `media`, `tasks`, and `web`.

Commit: `483ae74`. Verified by extracting the block and parsing it.
