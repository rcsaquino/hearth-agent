# The address guard misses the image client and can be bypassed by re-resolution

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-model`, `hearth-tools` |

## Observed

Two outbound paths can reach an address the product intends to block.

`ImageClient::generate` in `crates/hearth-model/src/media.rs` reads a response
from the configured image service. When the response carries `data[0].url`
instead of `b64_json`, it calls the blocking client on that URL and reads the
whole body with `bytes`. It never calls `fetch::validate_target`, and it has no
byte cap. The URL is data supplied by a remote service, so a compromised or
malicious endpoint can point Hearth at a loopback or private address and make it
download the response.

`fetch::validate_target` resolves the host with `to_socket_addrs` and checks
every returned address. `HttpFetch::fetch` then gives the original URL to
reqwest, which resolves the name again at connect time. A DNS name can return a
public address during the check and a loopback address for the connection.
`BrowserTool::open` has the same split in a different form. It validates the URL
and then hands it to Camofox, which resolves it after the check. The guard is a
best-effort check, not a property of the connection.

The solved `the-browser-bypasses-the-fetch-address-guard` issue added the check
to browser navigation. It did not cover the image client or the resolution
window.

## Consequence

The model can be induced by untrusted content to fetch local services, and the
image path can allocate a body with no limit. The shell tool is the deliberate
way to reach local addresses; these paths bypass that boundary.

## Verification

Point `ImageClient` at a fake service that returns a `url` field of
`http://127.0.0.1:<port>/secret`, where a local listener records the request.
`media` with `action: image` fetches it. For the fetch path, a DNS resolver that
answers public for the validation lookup and loopback for the connection shows
the bypass without changing the URL. The latter needs a controlled resolver and
is not covered by the existing private-address tests.

## Resolution

`hearth_model::net` now owns the address predicate, the URL check, and a
`public_resolver` that filters the addresses reqwest connects to. `ImageClient`
validates a service-supplied `data[0].url` with that predicate, downloads it
through a client carrying the resolver, and caps both the JSON answer and the
downloaded bytes at 32 MiB. `HttpFetch` installs the same resolver, so a name
that changes its answer between the check and the connection is refused at
connect time.

The browser still hands the URL to Camofox after the check; that remainder is
filed as `2026-11-17-the-browser-still-resolves-after-the-guard.md`.

Commits: `24b9c1f` (test), `1af6b6b` (fix). Verified by
`a_service_supplied_loopback_url_is_refused`, which serves a loopback URL from
the image service and requires an error with zero connections to the listener.
