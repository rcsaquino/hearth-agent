//! Optional tool groups discovered through `tool_search`.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use hearth_model::ToolSpec;
use hearth_tools::{Tool, ToolError, ToolOutput};
use serde_json::{Value, json};

/// One optional group of tools.
#[derive(Clone)]
pub struct ToolGroup {
    pub name: String,
    pub description: String,
    pub tools: Vec<Arc<dyn Tool>>,
}

impl ToolGroup {
    /// Build one group. An empty group is honest but unavailable.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            tools,
        }
    }

    /// Whether the group has any callable tool in this build.
    pub fn available(&self) -> bool {
        !self.tools.is_empty()
    }

    /// The group's tool declarations.
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(|tool| tool.spec()).collect()
    }

    fn find(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.iter().find(|tool| tool.spec().name == name)
    }
}

/// The optional groups known to this runtime.
#[derive(Clone, Default)]
pub struct ToolGroups {
    groups: BTreeMap<String, Arc<ToolGroup>>,
}

impl ToolGroups {
    /// Build the group registry. Later groups with the same name replace earlier ones.
    pub fn new(groups: Vec<ToolGroup>) -> Self {
        let groups = groups
            .into_iter()
            .map(|group| (group.name.clone(), Arc::new(group)))
            .collect();
        Self { groups }
    }

    /// One group by exact name.
    pub fn get(&self, name: &str) -> Option<&Arc<ToolGroup>> {
        self.groups.get(name)
    }

    /// Groups matching any word in the query. An empty query returns all.
    pub fn search(&self, query: &str) -> Vec<&ToolGroup> {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|term| {
                term.trim_matches(|character: char| !character.is_alphanumeric())
                    .to_ascii_lowercase()
            })
            .filter(|term| term.chars().count() >= 2)
            .collect();
        self.groups
            .values()
            .map(|group| group.as_ref())
            .filter(|group| {
                if terms.is_empty() {
                    return true;
                }
                let name = group.name.to_ascii_lowercase();
                let description = group.description.to_ascii_lowercase();
                terms
                    .iter()
                    .any(|term| name.contains(term) || description.contains(term))
            })
            .collect()
    }

    /// Tool declarations for the selected groups, in stable name order.
    pub fn selected_specs(&self, selection: &HashSet<String>) -> Vec<ToolSpec> {
        let mut names: Vec<&String> = selection.iter().collect();
        names.sort();
        names
            .into_iter()
            .filter_map(|name| self.groups.get(name))
            .flat_map(|group| group.specs())
            .collect()
    }

    /// Call a tool that belongs to one of the selected groups.
    pub fn call_selected(
        &self,
        name: &str,
        arguments: &Value,
        selection: &HashSet<String>,
    ) -> Option<Result<ToolOutput, ToolError>> {
        let mut names: Vec<&String> = selection.iter().collect();
        names.sort();
        for group_name in names {
            if let Some(group) = self.groups.get(group_name)
                && let Some(tool) = group.find(name)
            {
                return Some(tool.call(arguments));
            }
        }
        None
    }

    /// Estimated schema tokens for a list of tool specs.
    pub fn schema_tokens(specs: &[ToolSpec]) -> i64 {
        let values: Vec<Value> = specs
            .iter()
            .map(|spec| {
                json!({
                    "name": spec.name,
                    "description": spec.description,
                    "parameters": spec.parameters,
                })
            })
            .collect();
        hearth_model::estimate_tokens(&serde_json::to_string(&values).unwrap_or_default())
    }
}

/// The model-facing discovery tool.
pub struct ToolSearchTool {
    groups: Arc<ToolGroups>,
    selection: Arc<Mutex<HashSet<String>>>,
}

impl ToolSearchTool {
    /// Build the tool over the optional groups and the shared selection.
    pub fn new(groups: Arc<ToolGroups>, selection: Arc<Mutex<HashSet<String>>>) -> Self {
        Self { groups, selection }
    }
}

impl Tool for ToolSearchTool {
    fn spec(&self) -> ToolSpec {
        let mut description = "Find optional tool groups and load a group's schemas for this run. Use select to make a group's tools callable."
            .to_owned();
        let groups = self.groups.search("");
        if !groups.is_empty() {
            let index = groups
                .iter()
                .map(|group| format!("{}: {}", group.name, group.description))
                .collect::<Vec<_>>()
                .join("; ");
            description.push_str(" Available groups: ");
            description.push_str(&index);
            description.push('.');
        }
        ToolSpec {
            name: "tool_search".to_owned(),
            description,
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Short words to match group names and descriptions"},
                    "select": {"type": "string", "description": "An exact group name to load"}
                }
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        if let Some(name) = arguments.get("select").and_then(Value::as_str) {
            let Some(group) = self.groups.get(name) else {
                return Err(ToolError::invalid(
                    "unknown_group",
                    format!("unknown optional group: {name}"),
                ));
            };
            if !group.available() {
                return Err(ToolError::invalid(
                    "group_unavailable",
                    format!("optional group is not available in this build: {name}"),
                ));
            }
            let mut selection = self.selection.lock().map_err(|_| {
                ToolError::invalid("selection_poisoned", "tool selection is poisoned")
            })?;
            selection.insert(name.to_owned());
            let tools = group
                .specs()
                .into_iter()
                .map(|spec| spec.name)
                .collect::<Vec<_>>()
                .join(", ");
            return Ok(ToolOutput {
                content: format!("Selected optional group {name}: {tools}"),
                is_error: false,
            });
        }

        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let groups = self.groups.search(query);
        if groups.is_empty() {
            return Ok(ToolOutput {
                content: "No optional tool groups match that query.".to_owned(),
                is_error: false,
            });
        }
        let mut content = String::from("Optional tool groups:\n");
        for group in groups {
            let state = if group.available() {
                "available"
            } else {
                "unavailable in this build"
            };
            content.push_str(&format!(
                "- {}: {} ({state})\n",
                group.name, group.description
            ));
        }
        Ok(ToolOutput {
            content,
            is_error: false,
        })
    }
}
