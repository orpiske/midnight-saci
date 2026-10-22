//! The immutable tool registry: input schemas and argument validation.
//!
//! The registry is built once from a validated [`Config`] and never changes.
//! [`ScriptTool::arguments`] turns call arguments into the script's argument
//! list. It checks every value again on each call: the advertised schema is
//! only a hint to the client.

use std::path::PathBuf;
use std::time::Duration;

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::config::{ArgumentConfig, Binding, Config, ConfigError};

/// Longest accepted argument value, in bytes.
pub const MAX_VALUE_BYTES: usize = 4096;

/// A configured script.
#[derive(Debug, Clone)]
pub struct ScriptTool {
    /// Tool name.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// Absolute path of the script.
    pub executable: PathBuf,
    /// Deadline for one call.
    pub timeout: Duration,
    /// The tool does not change anything.
    pub read_only: bool,
    parameters: Vec<Parameter>,
}

#[derive(Debug, Clone)]
struct Parameter {
    name: String,
    description: Option<String>,
    required: bool,
    allowed: Option<Vec<String>>,
    pattern: Option<Regex>,
    binding: Binding,
}

/// All configured scripts, in configuration order.
#[derive(Debug, Clone)]
pub struct Registry {
    tools: Vec<ScriptTool>,
}

impl Registry {
    /// Builds the registry from a configuration.
    ///
    /// # Errors
    ///
    /// Fails when a pattern does not compile. [`Config::validate`] already
    /// rejects such configurations.
    pub fn new(config: &Config) -> Result<Self, ConfigError> {
        let mut tools = Vec::with_capacity(config.tools.len());
        for tool in &config.tools {
            let parameters = tool
                .arguments
                .iter()
                .map(Parameter::new)
                .collect::<Result<_, _>>()?;
            tools.push(ScriptTool {
                name: tool.name.clone(),
                description: tool.description.clone(),
                executable: tool.executable.clone(),
                timeout: Duration::from_secs(
                    tool.timeout_seconds
                        .unwrap_or(config.execution.default_timeout_seconds),
                ),
                read_only: tool.read_only,
                parameters,
            });
        }
        Ok(Self { tools })
    }

    /// Every tool, in configuration order.
    #[must_use]
    pub fn tools(&self) -> &[ScriptTool] {
        &self.tools
    }

    /// The tool with this name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ScriptTool> {
        self.tools.iter().find(|tool| tool.name == name)
    }
}

impl ScriptTool {
    /// The JSON Schema of the tool input.
    #[must_use]
    pub fn input_schema(&self) -> Map<String, Value> {
        let mut properties = Map::new();
        for parameter in &self.parameters {
            let mut property = Map::new();
            property.insert("type".to_owned(), json!("string"));
            if let Some(description) = &parameter.description {
                property.insert("description".to_owned(), json!(description));
            }
            if let Some(allowed) = &parameter.allowed {
                property.insert("enum".to_owned(), json!(allowed));
            }
            if let Some(pattern) = &parameter.pattern {
                property.insert("pattern".to_owned(), json!(pattern.as_str()));
            }
            property.insert("maxLength".to_owned(), json!(MAX_VALUE_BYTES));
            properties.insert(parameter.name.clone(), Value::Object(property));
        }
        let required: Vec<&str> = self
            .parameters
            .iter()
            .filter(|parameter| parameter.required)
            .map(|parameter| parameter.name.as_str())
            .collect();
        let mut schema = Map::new();
        schema.insert("type".to_owned(), json!("object"));
        schema.insert("properties".to_owned(), Value::Object(properties));
        schema.insert("required".to_owned(), json!(required));
        schema.insert("additionalProperties".to_owned(), json!(false));
        schema
    }

    /// Validates call arguments and returns the script's argument list, in
    /// configuration order.
    ///
    /// # Errors
    ///
    /// Returns a message for the caller when an argument is unknown, missing
    /// or not permitted.
    pub fn arguments(&self, input: Option<&Map<String, Value>>) -> Result<Vec<String>, String> {
        let empty = Map::new();
        let input = input.unwrap_or(&empty);
        if let Some(name) = input
            .keys()
            .find(|name| !self.parameters.iter().any(|p| &p.name == *name))
        {
            return Err(format!("unknown argument `{name}`"));
        }
        let mut argv = Vec::new();
        for parameter in &self.parameters {
            let Some(value) = input.get(&parameter.name) else {
                if parameter.required {
                    return Err(format!("missing required argument `{}`", parameter.name));
                }
                continue;
            };
            let value = parameter
                .check(value)
                .map_err(|reason| format!("argument `{}`: {reason}", parameter.name))?;
            if let Binding::Option { flag } = &parameter.binding {
                argv.push(flag.clone());
            }
            argv.push(value.to_owned());
        }
        Ok(argv)
    }
}

impl Parameter {
    fn new(argument: &ArgumentConfig) -> Result<Self, ConfigError> {
        let pattern = match &argument.pattern {
            Some(pattern) => {
                Some(Regex::new(pattern).map_err(|error| ConfigError::Invalid(error.to_string()))?)
            }
            None => None,
        };
        Ok(Self {
            name: argument.name.clone(),
            description: argument.description.clone(),
            required: argument.required,
            allowed: argument.allowed.clone(),
            pattern,
            binding: argument.binding.clone(),
        })
    }

    fn check<'a>(&self, value: &'a Value) -> Result<&'a str, String> {
        let Value::String(text) = value else {
            return Err("must be a string".to_owned());
        };
        check_value_text(text).map_err(str::to_owned)?;
        if let Some(allowed) = &self.allowed
            && !allowed.contains(text)
        {
            return Err(format!("must be one of: {}", allowed.join(", ")));
        }
        if let Some(pattern) = &self.pattern
            && !pattern.is_match(text)
        {
            return Err(format!("must match `{}`", pattern.as_str()));
        }
        Ok(text)
    }
}

/// Checks that apply to every value: not too long, no NUL character, and no
/// leading `-`, so that a value cannot act as a flag of the script.
///
/// # Errors
///
/// Returns the reason the value is not permitted.
pub fn check_value_text(text: &str) -> Result<(), &'static str> {
    if text.len() > MAX_VALUE_BYTES {
        Err("is longer than 4096 bytes")
    } else if text.contains('\0') {
        Err("contains a NUL character")
    } else if text.starts_with('-') {
        Err("must not start with `-`")
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::config::{ArgumentType, ExecutionConfig, ServerConfig, ToolConfig};

    fn argument(name: &str, required: bool, binding: Binding) -> ArgumentConfig {
        ArgumentConfig {
            name: name.to_owned(),
            description: None,
            kind: ArgumentType::String,
            required,
            allowed: None,
            pattern: None,
            binding,
        }
    }

    fn option(flag: &str) -> Binding {
        Binding::Option {
            flag: flag.to_owned(),
        }
    }

    fn deploy() -> ScriptTool {
        let mut environment = argument("environment", true, option("--environment"));
        environment.allowed = Some(vec!["staging".to_owned(), "production".to_owned()]);
        let mut version = argument("version", true, option("--version"));
        version.pattern = Some("^[a-zA-Z0-9._-]+$".to_owned());
        let target = argument("target", false, Binding::Positional {});
        let config = Config {
            server: ServerConfig::default(),
            execution: ExecutionConfig {
                max_concurrent: 1,
                default_timeout_seconds: 7,
                max_output_bytes: 10,
                working_directory: "/".into(),
                environment: std::collections::BTreeMap::new(),
            },
            tools: vec![ToolConfig {
                name: "deploy".to_owned(),
                description: "Deploy".to_owned(),
                executable: "/opt/deploy.sh".into(),
                timeout_seconds: None,
                read_only: false,
                arguments: vec![target, environment, version],
            }],
        };
        Registry::new(&config)
            .unwrap()
            .get("deploy")
            .unwrap()
            .clone()
    }

    fn call(tool: &ScriptTool, input: &Value) -> Result<Vec<String>, String> {
        tool.arguments(input.as_object())
    }

    #[test]
    fn the_schema_lists_parameters_and_rejects_others() {
        let tool = deploy();
        assert_eq!(tool.timeout, Duration::from_secs(7));
        let schema = Value::Object(tool.input_schema());
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["environment", "version"]));
        assert_eq!(
            schema["properties"]["environment"]["enum"],
            json!(["staging", "production"])
        );
        assert_eq!(
            schema["properties"]["version"]["pattern"],
            "^[a-zA-Z0-9._-]+$"
        );
        assert_eq!(schema["properties"]["target"]["type"], "string");
        assert!(schema["properties"]["target"].get("enum").is_none());
    }

    #[test]
    fn arguments_follow_configuration_order_not_input_order() {
        let tool = deploy();
        let argv = call(
            &tool,
            &json!({"version": "1.4.2", "target": "web", "environment": "staging"}),
        )
        .unwrap();
        assert_eq!(
            argv,
            ["web", "--environment", "staging", "--version", "1.4.2"]
        );
    }

    #[test]
    fn omitted_optional_arguments_are_not_passed() {
        let argv = call(
            &deploy(),
            &json!({"environment": "staging", "version": "1"}),
        )
        .unwrap();
        assert_eq!(argv, ["--environment", "staging", "--version", "1"]);
    }

    fn rejects(input: &Value, expected: &str) {
        let error = call(&deploy(), input).unwrap_err();
        assert!(error.contains(expected), "`{error}` lacks `{expected}`");
    }

    #[test]
    fn missing_unknown_and_mistyped_arguments_are_rejected() {
        rejects(
            &json!({"version": "1"}),
            "missing required argument `environment`",
        );
        rejects(
            &json!({"environment": "staging", "version": 1}),
            "must be a string",
        );
        rejects(
            &json!({"environment": "staging", "version": "1", "target": null}),
            "must be a string",
        );
        rejects(
            &json!({"environment": "staging", "version": "1", "extra": "x"}),
            "unknown argument `extra`",
        );
        assert!(deploy().arguments(None).is_err());
    }

    #[test]
    fn values_outside_the_enum_pattern_or_value_rules_are_rejected() {
        rejects(
            &json!({"environment": "dev", "version": "1"}),
            "must be one of",
        );
        rejects(
            &json!({"environment": "staging", "version": "1; rm -rf /"}),
            "must match",
        );
        let target =
            |value: &str| json!({"environment": "staging", "version": "1", "target": value});
        rejects(&target("--force"), "must not start with `-`");
        rejects(&target("a\u{0}b"), "NUL");
        rejects(&target(&"x".repeat(5000)), "longer");
    }

    #[test]
    fn shell_metacharacters_stay_one_literal_argument() {
        let value = "$(id) `id` ; | & > /tmp/x 'q' \"d\" *";
        let argv = call(
            &deploy(),
            &json!({"environment": "staging", "version": "1", "target": value}),
        )
        .unwrap();
        assert_eq!(argv[0], value);
        assert_eq!(argv.len(), 5);
    }
}
