//! The tool boundary: specs, calls, results, and a small registry.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use hearth_model::ToolSpec;
use serde_json::{Value, json};
use thiserror::Error;

/// The result of one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

/// A structured tool failure.
#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct ToolError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub detail: Option<String>,
}

impl ToolError {
    /// A caller error that a retry will not fix.
    pub fn invalid(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
            detail: None,
        }
    }

    /// A temporary failure.
    pub fn temporary(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: true,
            detail: None,
        }
    }

    /// Attach extra detail.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Whether the same call may succeed later.
    pub fn retryable(&self) -> bool {
        self.retryable
    }

    /// A compact JSON view for a tool result.
    pub fn to_json(&self) -> Value {
        let mut error = json!({
            "code": self.code,
            "message": self.message,
            "retryable": self.retryable,
        });
        if let Some(detail) = &self.detail {
            error["detail"] = Value::String(detail.clone());
        }
        json!({ "error": error })
    }
}

impl From<std::io::Error> for ToolError {
    fn from(error: std::io::Error) -> Self {
        Self::temporary("io", error.to_string())
    }
}

/// Something the model can call.
pub trait Tool: Send + Sync {
    /// The declaration sent to the model.
    fn spec(&self) -> ToolSpec;

    /// Run one call.
    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError>;
}

/// A set of named tools.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// Build a registry from tools. A later tool with the same name replaces an earlier one.
    pub fn new(tools: Vec<Arc<dyn Tool>>) -> Self {
        let mut registry = Self::default();
        for tool in tools {
            registry.tools.insert(tool.spec().name, tool);
        }
        registry
    }

    /// All specs in stable name order.
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|tool| tool.spec()).collect()
    }

    /// Call a tool, or return None when the name is unknown.
    pub fn call(
        &self,
        name: &str,
        arguments: &Value,
    ) -> Option<Result<ToolOutput, ToolError>> {
        self.tools.get(name).map(|tool| tool.call(arguments))
    }

    /// Whether the registry has no tools.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl fmt::Debug for ToolRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolRegistry")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}
