//! The optional browser tool: the local Camofox REST service.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hearth_model::ToolSpec;
use reqwest::blocking::{Client, Response};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::tool::{Tool, ToolError, ToolOutput};
use crate::workspace::Workspace;

/// The largest snapshot or page text returned in one tool result.
const INLINE_CAP: usize = 100_000;
/// The session label sent with every Camofox request.
const DEFAULT_USER: &str = "hearth";

/// Controls the local Camofox browser through its REST API.
pub struct BrowserTool {
    base_url: String,
    user_id: String,
    workspace: Workspace,
    client: Client,
}

impl BrowserTool {
    /// Build the tool against a local Camofox service.
    pub fn new(
        base_url: impl Into<String>,
        workspace: impl Into<PathBuf>,
        timeout: Duration,
    ) -> Result<Self, ToolError> {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| ToolError::invalid("browser_config", error.to_string()))?;
        let root = workspace.into();
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            user_id: DEFAULT_USER.to_owned(),
            workspace: Workspace::root_only(root),
            client,
        })
    }

    fn tab_url(&self, tab_id: &str, endpoint: &str) -> String {
        format!("{}/tabs/{tab_id}/{endpoint}", self.base_url)
    }

    fn send(
        &self,
        method: Method,
        url: String,
        body: Option<Value>,
    ) -> Result<Response, ToolError> {
        let mut request = self.client.request(method, url);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().map_err(browser_error)?;
        if !response.status().is_success() {
            let status = response.status();
            let text: String = response
                .text()
                .unwrap_or_default()
                .chars()
                .take(300)
                .collect();
            let retryable = status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS;
            let message = format!("Camofox returned {status}: {text}");
            return Err(if retryable {
                ToolError::temporary("browser_error", message)
            } else {
                ToolError::invalid("browser_error", message)
            });
        }
        Ok(response)
    }

    fn json_response(&self, response: Response) -> Result<Value, ToolError> {
        response
            .json()
            .map_err(|error| ToolError::invalid("browser_protocol", error.to_string()))
    }

    fn open(&self, url: &str) -> Result<ToolOutput, ToolError> {
        let body = json!({
            "userId": self.user_id,
            "sessionKey": self.user_id,
            "url": url,
        });
        let response = self.send(Method::POST, format!("{}/tabs", self.base_url), Some(body))?;
        let value = self.json_response(response)?;
        let tab = value
            .get("tabId")
            .or_else(|| value.get("tab_id"))
            .or_else(|| value.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("browser_protocol", "Camofox returned no tab id"))?;
        let title = value.get("title").and_then(Value::as_str).unwrap_or("");
        Ok(ToolOutput {
            content: format!("opened tab {tab}\ntitle: {title}"),
            is_error: false,
        })
    }

    fn read(&self, tab_id: &str) -> Result<ToolOutput, ToolError> {
        let response = self
            .client
            .get(self.tab_url(tab_id, "snapshot"))
            .query(&[("userId", self.user_id.as_str()), ("format", "text")])
            .send()
            .map_err(browser_error)?;
        if !response.status().is_success() {
            return Err(status_error(response));
        }
        let body = response
            .text()
            .map_err(|error| ToolError::temporary("browser_error", error.to_string()))?;
        let text = match serde_json::from_str::<Value>(&body) {
            Ok(value) => value
                .get("snapshot")
                .or_else(|| value.get("text"))
                .or_else(|| value.get("content"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or(body),
            Err(_) => body,
        };
        Ok(ToolOutput {
            content: cap(text),
            is_error: false,
        })
    }

    fn click(&self, tab_id: &str, reference: &str) -> Result<ToolOutput, ToolError> {
        let body = json!({"userId": self.user_id, "ref": reference});
        self.send(Method::POST, self.tab_url(tab_id, "click"), Some(body))?;
        Ok(ToolOutput {
            content: format!("clicked {reference}"),
            is_error: false,
        })
    }

    fn type_text(
        &self,
        tab_id: &str,
        reference: &str,
        text: &str,
    ) -> Result<ToolOutput, ToolError> {
        let body = json!({
            "userId": self.user_id,
            "ref": reference,
            "text": text,
            "clear": false,
            "submit": false,
        });
        self.send(Method::POST, self.tab_url(tab_id, "type"), Some(body))?;
        Ok(ToolOutput {
            content: format!("typed into {reference}"),
            is_error: false,
        })
    }

    fn screenshot(
        &self,
        tab_id: &str,
        path: Option<&str>,
        full_page: bool,
    ) -> Result<ToolOutput, ToolError> {
        let response = self
            .client
            .get(self.tab_url(tab_id, "screenshot"))
            .query(&[
                ("userId", self.user_id.as_str()),
                ("fullPage", if full_page { "true" } else { "false" }),
            ])
            .send()
            .map_err(browser_error)?;
        if !response.status().is_success() {
            return Err(status_error(response));
        }
        let bytes = response
            .bytes()
            .map_err(|error| ToolError::temporary("browser_error", error.to_string()))?;
        let path = match path {
            Some(path) if !path.trim().is_empty() => path.trim().to_owned(),
            _ => {
                let stamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                format!("browser-{stamp}.png")
            }
        };
        self.workspace.write_bytes(&path, &bytes)?;
        Ok(ToolOutput {
            content: format!("screenshot: {path} ({} bytes)", bytes.len()),
            is_error: false,
        })
    }
}

impl Tool for BrowserTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "browser".to_owned(),
            description: "Control the local Camofox browser: open a URL, read the page snapshot, click or type by ref, or take a screenshot.".to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["open", "read", "click", "type", "screenshot"]},
                    "url": {"type": "string"},
                    "tab_id": {"type": "string"},
                    "ref": {"type": "string"},
                    "text": {"type": "string"},
                    "path": {"type": "string", "description": "Screenshot path inside the workspace"},
                    "full_page": {"type": "boolean"}
                },
                "required": ["action"]
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "action is required"))?;
        match action {
            "open" => {
                let url = arguments
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ToolError::invalid("invalid_arguments", "url is required for open")
                    })?;
                self.open(url)
            }
            "read" => {
                let tab = required(arguments, "tab_id")?;
                self.read(tab)
            }
            "click" => {
                let tab = required(arguments, "tab_id")?;
                let reference = required(arguments, "ref")?;
                self.click(tab, reference)
            }
            "type" => {
                let tab = required(arguments, "tab_id")?;
                let reference = required(arguments, "ref")?;
                let text = required(arguments, "text")?;
                self.type_text(tab, reference, text)
            }
            "screenshot" => {
                let tab = required(arguments, "tab_id")?;
                let path = arguments.get("path").and_then(Value::as_str);
                let full_page = arguments
                    .get("full_page")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.screenshot(tab, path, full_page)
            }
            other => Err(ToolError::invalid(
                "unknown_action",
                format!("unknown browser action: {other}"),
            )),
        }
    }
}

fn required<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ToolError::invalid("invalid_arguments", format!("{key} is required")))
}

fn browser_error(error: reqwest::Error) -> ToolError {
    if error.is_timeout() {
        ToolError::temporary("browser_timeout", format!("Camofox timed out: {error}"))
    } else if error.is_connect() {
        ToolError::temporary(
            "browser_unavailable",
            format!("Camofox is not reachable: {error}"),
        )
    } else {
        ToolError::temporary("browser_error", error.to_string())
    }
}

fn status_error(response: Response) -> ToolError {
    let status = response.status();
    let text: String = response
        .text()
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect();
    let retryable = status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS;
    let message = format!("Camofox returned {status}: {text}");
    if retryable {
        ToolError::temporary("browser_error", message)
    } else {
        ToolError::invalid("browser_error", message)
    }
}

fn cap(mut text: String) -> String {
    if text.chars().count() > INLINE_CAP {
        let truncated: String = text.chars().take(INLINE_CAP).collect();
        text = format!("{truncated}\n[truncated at {INLINE_CAP} characters]");
    }
    text
}
