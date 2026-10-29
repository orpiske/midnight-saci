//! Checking call arguments against a tool's discovered input schema.
//!
//! Only the parts of JSON Schema that decide whether a call is well formed
//! at the top level are checked: required properties, property types, and
//! `additionalProperties: false`. The server remains the final judge.
//! [`fit_arguments`] shortens string values to the `maxLength` of their
//! property.

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

/// The text that ends a shortened value.
const TRUNCATED: &str = "\n[truncated]";

/// Shortens every top-level string argument that is longer than the
/// `maxLength` of its property in `tool`'s input schema, and returns the
/// names of the shortened arguments.
///
/// The limit is applied in bytes. A byte count is never smaller than the
/// character count that JSON Schema uses, so the result satisfies both. A
/// value is cut at a character boundary, at the last line break when one is
/// in the second half of the kept text, and ends with a `[truncated]` line.
pub fn fit_arguments(tool: &Tool, arguments: &mut Map<String, Value>) -> Vec<String> {
    let properties = tool
        .input_schema
        .get("properties")
        .and_then(Value::as_object);
    let mut shortened = Vec::new();
    for (name, value) in arguments.iter_mut() {
        let limit = properties
            .and_then(|properties| properties.get(name))
            .and_then(|property| property.get("maxLength"))
            .and_then(Value::as_u64)
            .and_then(|limit| usize::try_from(limit).ok());
        if let (Some(limit), Value::String(text)) = (limit, value)
            && text.len() > limit
        {
            *text = shorten(text, limit);
            shortened.push(name.clone());
        }
    }
    shortened
}

/// `text` cut to at most `limit` bytes, with the marker when it fits.
fn shorten(text: &str, limit: usize) -> String {
    let Some(room) = limit.checked_sub(TRUNCATED.len()) else {
        return text[..floor_boundary(text, limit)].to_owned();
    };
    let kept = &text[..floor_boundary(text, room)];
    let kept = match kept.rfind('\n') {
        Some(end) if end >= kept.len() / 2 => &kept[..end],
        _ => kept,
    };
    format!("{kept}{TRUNCATED}")
}

/// The largest character boundary of `text` that is not after `at`.
fn floor_boundary(text: &str, at: usize) -> usize {
    let mut end = at.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::Arc;

    use rmcp::model::Tool;
    use serde_json::{Value, json};

    use super::{check_arguments, fit_arguments};

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

    #[test]
    fn long_strings_are_cut_to_max_length() {
        let tool = tool(&json!({
            "type": "object",
            "properties": {
                "summary": {"type": "string", "maxLength": 52},
                "title": {"type": "string", "maxLength": 40},
                "body": {"type": "string"}
            }
        }));
        let summary = "- [passed] 1 first\n- [failed] 2 second\n- [passed] 3 third\n";
        let long = "x".repeat(100);
        let mut arguments = arguments(&json!({
            "summary": summary, "title": "short", "body": long
        }));
        assert_eq!(fit_arguments(&tool, &mut arguments), ["summary"]);
        // Cut after the last whole line that fits, then the marker.
        assert_eq!(
            arguments["summary"],
            "- [passed] 1 first\n- [failed] 2 second\n[truncated]"
        );
        assert_eq!(arguments["title"], "short");
        assert_eq!(arguments["body"], json!(long));
    }

    #[test]
    fn cuts_keep_whole_characters() {
        let tool = tool(&json!({
            "properties": {"a": {"maxLength": 16}, "b": {"maxLength": 5}}
        }));
        let mut arguments = arguments(&json!({"a": "é".repeat(20), "b": "ééé"}));
        assert_eq!(fit_arguments(&tool, &mut arguments).len(), 2);
        // 16 bytes leave room for 4 bytes of text: two characters.
        assert_eq!(arguments["a"], "éé\n[truncated]");
        // No room for the marker: the value is only cut.
        assert_eq!(arguments["b"], "éé");
    }
}
