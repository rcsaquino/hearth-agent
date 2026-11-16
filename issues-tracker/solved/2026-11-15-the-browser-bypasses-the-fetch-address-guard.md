# The browser tool bypasses the address guard the fetch tool enforces

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-15 |
| Area | `hearth-tools` |

## Observed

`web_fetch` refuses loopback, private, link-local, and reserved addresses before
any provider runs, and it re-validates every redirect. The browser tool passes
the model's URL to Camofox unchanged.

`crates/hearth-tools/src/fetch.rs:101`:

```rust
validate_target(&parsed, self.allow_loopback)
    .map_err(|error| ToolError::invalid("blocked_address", error.message))?;
```

`crates/hearth-tools/src/browser.rs:87`:

```rust
fn open(&self, url: &str) -> Result<ToolOutput, ToolError> {
    let body = json!({
        "userId": self.user_id,
        "sessionKey": self.user_id,
        "url": url,
    });
    let response = self.send(Method::POST, format!("{}/tabs", self.base_url), Some(body))?;
```

`browser::read` then returns the rendered page text. A model can read
`http://127.0.0.1:<port>/` or `http://169.254.169.254/` through the browser
group while `web_fetch` refuses the same URL.

## Consequence

The two web tools enforce different boundaries, and the browser group is
described as "web pages that plain fetch cannot reach"
(`crates/hearth-runtime/src/tools.rs:165`), which reads as an invitation to the
difference. The practical capability is already available through `shell`, which
is not sandboxed, so this is not a new escape. It is an undocumented asymmetry in
an operator-facing guard, and a reader of the fetch guard will assume the
browser obeys it.

## What should happen

Either the browser applies the same target check, or the exception is stated
where the fetch guard is documented, with the reason it is safe under the
product's yolo model. The choice is a product decision; the current state states
neither.

## Verification

Read the two call paths. A test with a fake Camofox server and a loopback URL
shows `browser` sends it; the same URL through `web_fetch` returns
`blocked_address`.

## Resolution

`BrowserTool::open` parses the target and runs the fetch tool's `validate_target`
before contacting Camofox, returning the same `blocked_address` code. The guard
runs on the page the model names, not on the local Camofox base URL. `shell`
remains the deliberate way to reach a local service.

Commits: `1cb29a9` (test), `59954b3` (fix). Verified by
`a_private_address_is_refused_before_camofox_sees_it` with a fake Camofox server,
which records no tab request for a loopback URL. The other browser tests now
name a public literal address so the guard needs no resolver.
