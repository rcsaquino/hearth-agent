# The browser hands a validated name to Camofox, which resolves it again

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-tools` |

## Observed

`BrowserTool::open` calls `fetch::validate_target` on the parsed URL and then
sends the original URL to Camofox. Camofox resolves the name when it connects,
so a name that answers with a public address during the check and a loopback or
private address for the navigation passes the guard.

The direct fetch path no longer has this window because `hearth_model::net`
installs `public_resolver` on the HTTP client. The browser has no equivalent,
because the connection is made by the Camofox process.

## Wanted

Either make the browser reach only an address the guard already approved, for
example by resolving once and giving Camofox the approved address with the
original host, or state in `docs/SPEC.md` and `SECURITY.md` that browser
navigation is a best-effort check and name the local service as the boundary.

## Verification

Use a controlled resolver that answers public for the `validate_target` lookup
and loopback for Camofox's navigation, then call `BrowserTool::open`. The request
reaches the loopback listener.
