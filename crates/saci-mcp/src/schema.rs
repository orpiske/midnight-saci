//! Checking call arguments against a tool's discovered input schema.
//!
//! Only the parts of JSON Schema that decide whether a call is well formed
//! at the top level are checked: required properties, property types, and
//! `additionalProperties: false`. The server remains the final judge.

use rmcp::model::Tool;
use serde_json::{Map, Value};

/// Checks `arguments` against `tool`'s input schema, describing every
/// problem found.
///
/// # Errors
///
/// The problems, joined into one message.
pub fn check_arguments(tool: &Tool, arguments: &Map<String, Value>) -> Result<(), String> {
    let schema = tool.input_schema.as_ref();
    let properties = schema.get("properties").and_then(Value::as_object);
    let mut problems = Vec::new();
    for name in required(schema) {
        if !arguments.contains_key(name) {
            problems.push(format!("`{name}` is required"));
        }
    }
    for (name, value) in arguments {
        match properties.and_then(|properties| properties.get(name)) {
            Some(property) => {
                if let Some(expected) = mismatch(property, value) {
                    problems.push(format!("`{name}` must be {expected}"));
                }
            }
            None if schema.get("additionalProperties") == Some(&Value::Bool(false)) => {
                problems.push(format!("`{name}` is not an argument of `{}`", tool.name));
            }
            None => {}
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

fn required(schema: &Map<String, Value>) -> impl Iterator<Item = &str> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

/// The expected type when `value` does not have one of the types `property`
/// allows.
fn mismatch(property: &Value, value: &Value) -> Option<String> {
    let types: Vec<&str> = match property.get("type") {
        Some(Value::String(name)) => vec![name.as_str()],
        Some(Value::Array(names)) => names.iter().filter_map(Value::as_str).collect(),
        _ => return None,
    };
    if types.iter().any(|name| has_type(value, name)) {
        None
    } else {
        Some(format!("of type {}", types.join(" or ")))
    }
}

fn has_type(value: &Value, name: &str) -> bool {
    match name {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::Arc;

    use rmcp::model::Tool;
    use serde_json::{Value, json};

    use super::check_arguments;

    fn tool(schema: &Value) -> Tool {
        let Value::Object(schema) = schema.clone() else {
            unreachable!()
        };
        Tool::new("create_issue", "", Arc::new(schema))
    }

    fn arguments(value: &Value) -> serde_json::Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn arguments_are_checked_against_the_schema() {
        let tool = tool(&json!({
            "type": "object",
            "properties": {"title": {"type": "string"}, "priority": {"type": ["integer", "null"]}},
            "required": ["title"],
            "additionalProperties": false
        }));
        assert!(check_arguments(&tool, &arguments(&json!({"title": "x"}))).is_ok());
        let error = check_arguments(
            &tool,
            &arguments(&json!({"priority": "high", "labels": []})),
        )
        .unwrap_err();
        assert_eq!(
            error,
            "`title` is required; `priority` must be of type integer or null; \
             `labels` is not an argument of `create_issue`"
        );
    }

    #[test]
    fn open_schemas_accept_extra_arguments() {
        let tool = tool(&json!({"type": "object"}));
        assert!(check_arguments(&tool, &arguments(&json!({"any": 1}))).is_ok());
    }
}
