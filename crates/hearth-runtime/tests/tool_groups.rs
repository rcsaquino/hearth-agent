use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use hearth_model::ToolSpec;
use hearth_runtime::tool_groups::{ToolGroup, ToolGroups, ToolSearchTool};
use hearth_tools::{Tool, ToolError, ToolOutput};
use serde_json::{Value, json};

struct Echo;

impl Tool for Echo {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".to_owned(),
            description: "Echo the text argument.".to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }),
        }
    }

    fn call(&self, arguments: &Value) -> Result<ToolOutput, ToolError> {
        let text = arguments
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid("invalid_arguments", "text is required"))?;
        Ok(ToolOutput {
            content: text.to_owned(),
            is_error: false,
        })
    }
}

fn groups() -> ToolGroups {
    ToolGroups::new(vec![
        ToolGroup::new(
            "test",
            "A test group with one callable tool.",
            vec![Arc::new(Echo)],
        ),
        ToolGroup::new("empty", "A group with no tools yet.", Vec::new()),
    ])
}

#[test]
fn search_lists_groups_and_marks_unavailable_ones() {
    let groups = groups();
    let selection = Arc::new(Mutex::new(HashSet::new()));
    let tool = ToolSearchTool::new(Arc::new(groups), selection);

    let result = tool
        .call(&json!({"query": "test"}))
        .expect("search succeeds");
    assert!(result.content.contains("test"), "{}", result.content);
    assert!(
        !result.content.contains("missing"),
        "unknown names must not appear"
    );

    let all = tool.call(&json!({"query": ""})).expect("search all");
    assert!(all.content.contains("empty"), "{}", all.content);
    assert!(
        all.content.contains("unavailable"),
        "empty groups must be marked unavailable: {}",
        all.content
    );
}

#[test]
fn selecting_a_group_adds_its_specs_and_calls_are_confined() {
    let groups = Arc::new(groups());
    let selection = Arc::new(Mutex::new(HashSet::new()));
    let tool = ToolSearchTool::new(Arc::clone(&groups), Arc::clone(&selection));

    assert!(
        groups
            .selected_specs(&selection.lock().expect("selection"))
            .is_empty()
    );

    let selected = tool
        .call(&json!({"select": "test"}))
        .expect("select succeeds");
    assert!(selected.content.contains("echo"), "{}", selected.content);

    let specs = groups.selected_specs(&selection.lock().expect("selection"));
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].name, "echo");

    let output = groups
        .call_selected(
            "echo",
            &json!({"text": "hello"}),
            &selection.lock().expect("selection"),
        )
        .expect("the selected tool exists")
        .expect("the call succeeds");
    assert_eq!(output.content, "hello");
}

#[test]
fn unselected_groups_stay_out_and_bad_selections_are_errors() {
    let groups = Arc::new(groups());
    let selection = Arc::new(Mutex::new(HashSet::new()));
    let tool = ToolSearchTool::new(Arc::clone(&groups), selection);

    let error = tool
        .call(&json!({"select": "missing"}))
        .expect_err("unknown groups must fail");
    assert_eq!(error.code, "unknown_group");

    let error = tool
        .call(&json!({"select": "empty"}))
        .expect_err("empty groups must fail");
    assert_eq!(error.code, "group_unavailable");

    assert!(
        groups.selected_specs(&HashSet::new()).is_empty(),
        "nothing may be selected"
    );
}
