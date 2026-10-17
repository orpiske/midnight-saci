//! A stateless scripted model for demonstrations: the reply depends only on
//! the conversation it is sent, so any number of runs can share one server.
//!
//! A playbook maps unit titles to the tool calls the model makes for them,
//! in order:
//!
//! ```json
//! {"units": {"Check service health": [
//!     {"tool": "http", "arguments": {"method": "GET", "url": "${SERVICE_URL}/health"}},
//!     {"tool": "assess", "arguments": {"outcome": "passed", "criteria": [
//!         {"criterion": 0, "status": "satisfied", "basis": "mechanical",
//!          "evidence": ["$last"], "explanation": "status 200"}],
//!      "explanation": "the service is healthy"}}
//! ]}}
//! ```
//!
//! The unit is recognized by the `Current unit: ... "<title>"` line of the
//! conversation; the position by the number of tool calls the assistant has
//! already made. The string `"$last"` anywhere in the arguments becomes the
//! ID of the latest evidence the conversation shows.
//!
//! A call with `"expect": "<text>"` is made with its `arguments` only when
//! the latest tool result contains that text, and with its `otherwise`
//! arguments when it does not. An assessment can so follow what the target
//! actually answered, for example failing when the service is down.

use serde_json::{Map, Value};

use crate::{text_reply, tool_calls_reply};

/// A parsed playbook.
#[derive(Debug, Clone)]
pub struct Playbook {
    units: Map<String, Value>,
}

impl Playbook {
    /// Parses a playbook document.
    ///
    /// # Errors
    ///
    /// Describes what is malformed.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let document: Value =
            serde_json::from_str(text).map_err(|error| format!("invalid playbook: {error}"))?;
        let units = document
            .get("units")
            .and_then(Value::as_object)
            .ok_or("a playbook needs a `units` object mapping unit titles to tool calls")?;
        for (title, calls) in units {
            let valid = calls.as_array().is_some_and(|calls| {
                calls.iter().all(|call| {
                    call["tool"].is_string()
                        && call["arguments"].is_object()
                        && (call.get("expect").is_none()
                            || (call["expect"].is_string() && call["otherwise"].is_object()))
                })
            });
            if !valid {
                return Err(format!(
                    "unit `{title}` needs a list of {{\"tool\": ..., \"arguments\": {{...}}}}, \
                     with `otherwise` arguments for each `expect`"
                ));
            }
        }
        Ok(Self {
            units: units.clone(),
        })
    }

    /// The Chat Completions reply to `request`, or why there is none.
    ///
    /// # Errors
    ///
    /// Names the unit or position the playbook does not cover.
    pub fn reply(&self, request: &Value) -> Result<Value, String> {
        let messages = request["messages"].as_array().cloned().unwrap_or_default();
        let title = current_unit(&messages)
            .ok_or("the conversation names no current unit (`Current unit: ... \"<title>\"`)")?;
        let calls = self
            .units
            .get(&title)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("the playbook has no entry for unit `{title}`"))?;
        let made = messages
            .iter()
            .filter(|message| message["role"] == "assistant")
            .filter_map(|message| message["tool_calls"].as_array())
            .map(Vec::len)
            .sum::<usize>();
        let Some(call) = calls.get(made) else {
            return Ok(text_reply("The playbook for this unit is complete."));
        };
        let answered =
            |expected: &str| last_result(&messages).is_some_and(|t| t.contains(expected));
        let mut arguments = match call["expect"].as_str() {
            Some(expected) if !answered(expected) => call["otherwise"].clone(),
            _ => call["arguments"].clone(),
        };
        if let Some(evidence) = last_evidence(&messages) {
            substitute(&mut arguments, &evidence);
        }
        let id = format!("call-{}", made + 1);
        let tool = call["tool"].as_str().unwrap_or_default();
        Ok(tool_calls_reply(&[(&id, tool, &arguments.to_string())]))
    }
}

/// The title on the conversation's last `Current unit:` line.
fn current_unit(messages: &[Value]) -> Option<String> {
    messages
        .iter()
        .filter(|message| message["role"] == "user")
        .filter_map(content)
        .flat_map(|text| {
            text.lines()
                .filter(|line| line.starts_with("Current unit:"))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .last()
        .and_then(|line| line.split('"').nth(1).map(str::to_owned))
}

/// The text of the latest tool result.
fn last_result(messages: &[Value]) -> Option<String> {
    messages
        .iter()
        .filter(|message| message["role"] == "tool")
        .filter_map(content)
        .next_back()
}

/// The ID in the latest `Evidence ev-N (` tool result.
fn last_evidence(messages: &[Value]) -> Option<String> {
    messages
        .iter()
        .filter(|message| message["role"] == "tool")
        .filter_map(content)
        .filter_map(|text| {
            let rest = text.strip_prefix("Evidence ")?;
            Some(rest.split_whitespace().next()?.to_owned())
        })
        .next_back()
}

/// Message text, whether sent as a string or as content parts.
fn content(message: &Value) -> Option<String> {
    match &message["content"] {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

fn substitute(value: &mut Value, evidence: &str) {
    match value {
        Value::String(text) if text == "$last" => *text = evidence.to_owned(),
        Value::Array(items) => items.iter_mut().for_each(|item| substitute(item, evidence)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|item| substitute(item, evidence)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::Playbook;
    use serde_json::{Value, json};

    const BOOK: &str = r#"{"units": {"Health": [
        {"tool": "http", "arguments": {"method": "GET", "url": "${URL}/health"}},
        {"tool": "assess", "arguments": {"outcome": "passed", "criteria": [{"evidence": ["$last"]}]}}
    ]}}"#;

    fn call(reply: &Value) -> (String, Value) {
        let call = &reply["choices"][0]["message"]["tool_calls"][0]["function"];
        let arguments = serde_json::from_str(call["arguments"].as_str().unwrap()).unwrap();
        (call["name"].as_str().unwrap().to_owned(), arguments)
    }

    #[test]
    fn replies_follow_the_conversation() {
        let book = Playbook::from_json(BOOK).unwrap();
        let prompt =
            "Plan context\nCurrent unit: Test 1.1 \"Health\" (phase \"P\", plan.md:5), attempt 1.";
        let mut messages = vec![json!({"role": "user", "content": prompt})];
        let (tool, arguments) = call(&book.reply(&json!({ "messages": messages })).unwrap());
        assert_eq!(
            (tool.as_str(), arguments["url"].as_str()),
            ("http", Some("${URL}/health"))
        );
        messages.push(json!({"role": "assistant", "tool_calls": [{"id": "call-1"}]}));
        messages.push(json!({"role": "tool", "content": "Evidence ev-7 (http, 3 ms). data"}));
        let (tool, arguments) = call(&book.reply(&json!({ "messages": messages })).unwrap());
        assert_eq!(tool, "assess");
        assert_eq!(arguments["criteria"][0]["evidence"], json!(["ev-7"]));
    }

    #[test]
    fn unknown_units_and_bad_playbooks_are_named() {
        let book = Playbook::from_json(BOOK).unwrap();
        let prompt = "Current unit: Step 2 \"Other\" (phase \"P\", plan.md:9), attempt 1.";
        let request = json!({"messages": [{"role": "user", "content": prompt}]});
        assert!(book.reply(&request).unwrap_err().contains("`Other`"));
        let error = Playbook::from_json(r#"{"units": {"X": [{"tool": "shell"}]}}"#).unwrap_err();
        assert!(error.contains("unit `X`"), "{error}");
    }

    #[test]
    fn expectations_choose_the_arguments_from_the_last_answer() {
        let book = Playbook::from_json(
            r#"{"units": {"Health": [
                {"tool": "assess", "expect": "status 200",
                 "arguments": {"outcome": "passed"}, "otherwise": {"outcome": "failed"}}
            ]}}"#,
        )
        .unwrap();
        let prompt = "Current unit: Test 1.1 \"Health\" (phase \"P\", plan.md:5), attempt 1.";
        let outcome = |result: &str| {
            let messages = json!([
                {"role": "user", "content": prompt},
                {"role": "tool", "content": result}
            ]);
            let reply = book.reply(&json!({ "messages": messages })).unwrap();
            call(&reply).1["outcome"].as_str().unwrap().to_owned()
        };
        assert_eq!(
            outcome("Evidence ev-1 (shell). stdout:\nok\nstatus 200"),
            "passed"
        );
        assert_eq!(
            outcome("Evidence ev-1 (shell). exit code: 7\nstatus 000"),
            "failed"
        );
        let missing = r#"{"units": {"X": [{"tool": "a", "arguments": {}, "expect": "y"}]}}"#;
        assert!(
            Playbook::from_json(missing)
                .unwrap_err()
                .contains("`otherwise`")
        );
    }
}
