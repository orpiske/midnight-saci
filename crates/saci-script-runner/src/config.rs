//! The YAML configuration and its startup checks.
//!
//! Every section rejects unknown fields. [`Config::load`] parses the file and
//! then checks it with [`Config::validate`], so a running server only ever
//! sees a configuration that passed every check.

use std::collections::{BTreeMap, HashSet};
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Upper bound for `max_concurrent`.
pub const MAX_CONCURRENT_LIMIT: usize = 256;
/// Upper bound for a timeout, in seconds (one hour).
pub const TIMEOUT_LIMIT_SECONDS: u64 = 3600;
/// Upper bound for `max_output_bytes` (16 MiB per stream).
pub const OUTPUT_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// A configuration that cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file cannot be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The configuration file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The file is not valid YAML for this format.
    #[error("cannot parse {path}: {source}")]
    Parse {
        /// The configuration file.
        path: PathBuf,
        /// The cause.
        source: serde_yaml::Error,
    },
    /// A value is not permitted.
    #[error("{0}")]
    Invalid(String),
}

fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::Invalid(message.into())
}

/// The whole configuration file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// HTTP listener settings.
    #[serde(default)]
    pub server: ServerConfig,
    /// Process execution limits and environment.
    pub execution: ExecutionConfig,
    /// The scripts exposed as tools, in the order `tools/list` shows them.
    pub tools: Vec<ToolConfig>,
}

/// The `server` section.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Listen address. Default `127.0.0.1:3000`.
    #[serde(default = "default_bind")]
    pub bind: SocketAddr,
    /// MCP endpoint path. Default `/mcp`.
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    /// Name of the environment variable that holds the bearer token. When
    /// set, every MCP request must send `Authorization: Bearer <token>`.
    #[serde(default)]
    pub auth_token_env: Option<String>,
    /// Accepted `Host` header values. Default: `localhost`, `127.0.0.1`
    /// and `::1`.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    /// Accepted `Origin` header values. Requests that send an `Origin` that
    /// is not in this list are rejected. Default: none.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            endpoint: default_endpoint(),
            auth_token_env: None,
            allowed_hosts: Vec::new(),
            allowed_origins: Vec::new(),
        }
    }
}

fn default_bind() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 3000))
}

fn default_endpoint() -> String {
    "/mcp".to_owned()
}

/// The `execution` section.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionConfig {
    /// Scripts that can run at the same time. Default 4.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    /// Deadline for a tool without `timeout_seconds`. Default 30.
    #[serde(default = "default_timeout")]
    pub default_timeout_seconds: u64,
    /// Bytes kept from each of stdout and stderr. Default 65536.
    #[serde(default = "default_max_output")]
    pub max_output_bytes: usize,
    /// Working directory of every script. Must be absolute.
    pub working_directory: PathBuf,
    /// The complete environment of every script. Nothing is inherited.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

const fn default_max_concurrent() -> usize {
    4
}

const fn default_timeout() -> u64 {
    30
}

const fn default_max_output() -> usize {
    65536
}

/// One script exposed as one MCP tool.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolConfig {
    /// Tool name.
    pub name: String,
    /// Tool description for the MCP client.
    pub description: String,
    /// Absolute path of the script.
    pub executable: PathBuf,
    /// Deadline for this tool. Default: `execution.default_timeout_seconds`.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    /// Marks the tool as read-only for clients. Default false.
    #[serde(default)]
    pub read_only: bool,
    /// Parameters, in the order their values are put on the command line.
    #[serde(default)]
    pub arguments: Vec<ArgumentConfig>,
}

/// A tool parameter.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArgumentConfig {
    /// Parameter name.
    pub name: String,
    /// Parameter description for the MCP client.
    #[serde(default)]
    pub description: Option<String>,
    /// Value type. Only `string` is supported.
    #[serde(rename = "type")]
    pub kind: ArgumentType,
    /// The call must supply the parameter. Default false.
    #[serde(default)]
    pub required: bool,
    /// The only permitted values.
    #[serde(default, rename = "enum")]
    pub allowed: Option<Vec<String>>,
    /// A regular expression the value must match.
    #[serde(default)]
    pub pattern: Option<String>,
    /// How the value goes on the command line.
    pub binding: Binding,
}

/// Supported parameter types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentType {
    /// A string value.
    String,
}

/// How a parameter value goes on the command line.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Binding {
    /// The fixed flag, then the value, as two arguments.
    Option {
        /// The flag, for example `--version`.
        flag: String,
    },
    /// The value alone.
    Positional {},
}

impl Config {
    /// Reads, parses and validates a configuration file.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or parsed, or a check fails.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        let config: Self = serde_yaml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Checks every value, including the files the configuration names.
    ///
    /// # Errors
    ///
    /// Returns the first problem found.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.server.validate()?;
        self.execution.validate()?;
        if self.tools.is_empty() {
            return Err(invalid("`tools` must name at least one script"));
        }
        let mut names = HashSet::new();
        for tool in &self.tools {
            if !names.insert(tool.name.as_str()) {
                return Err(invalid(format!("tool `{}` is defined twice", tool.name)));
            }
            tool.validate()
                .map_err(|error| invalid(format!("tool `{}`: {error}", tool.name)))?;
        }
        Ok(())
    }
}

impl ServerConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        let endpoint = &self.endpoint;
        let valid_path = endpoint.len() > 1
            && endpoint.starts_with('/')
            && endpoint[1..]
                .split('/')
                .all(|part| !part.is_empty() && part.chars().all(is_name_char));
        if !valid_path || endpoint == crate::server::HEALTH_PATH {
            return Err(invalid(format!(
                "`server.endpoint` `{endpoint}` must be a path such as `/mcp`, other than `{}`",
                crate::server::HEALTH_PATH
            )));
        }
        if let Some(name) = &self.auth_token_env
            && !is_env_name(name)
        {
            return Err(invalid(format!(
                "`server.auth_token_env` `{name}` is not a variable name"
            )));
        }
        if !self.bind.ip().is_loopback() && self.auth_token_env.is_none() {
            return Err(invalid(format!(
                "`server.bind` {} is not a loopback address: set `server.auth_token_env`",
                self.bind
            )));
        }
        Ok(())
    }
}

impl ExecutionConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        if !(1..=MAX_CONCURRENT_LIMIT).contains(&self.max_concurrent) {
            return Err(invalid(format!(
                "`execution.max_concurrent` must be 1 to {MAX_CONCURRENT_LIMIT}"
            )));
        }
        check_timeout(
            "execution.default_timeout_seconds",
            self.default_timeout_seconds,
        )?;
        if !(1..=OUTPUT_LIMIT_BYTES).contains(&self.max_output_bytes) {
            return Err(invalid(format!(
                "`execution.max_output_bytes` must be 1 to {OUTPUT_LIMIT_BYTES}"
            )));
        }
        let directory = &self.working_directory;
        if !directory.is_absolute() || !directory.is_dir() {
            return Err(invalid(format!(
                "`execution.working_directory` {} must be an absolute path to a directory",
                directory.display()
            )));
        }
        self.validate_environment()
    }

    fn validate_environment(&self) -> Result<(), ConfigError> {
        if let Some(name) = self.environment.keys().find(|name| !is_env_name(name)) {
            return Err(invalid(format!(
                "`execution.environment`: `{name}` is not a variable name"
            )));
        }
        if let Some((name, _)) = self
            .environment
            .iter()
            .find(|(_, value)| value.contains('\0'))
        {
            return Err(invalid(format!(
                "`execution.environment.{name}` contains a NUL character"
            )));
        }
        Ok(())
    }
}

impl ToolConfig {
    fn validate(&self) -> Result<(), String> {
        if !is_tool_name(&self.name) {
            return Err("the name must be 1 to 64 letters, digits, `_` or `-`".to_owned());
        }
        if self.description.trim().is_empty() {
            return Err("`description` is empty".to_owned());
        }
        if let Some(seconds) = self.timeout_seconds {
            check_timeout("timeout_seconds", seconds).map_err(|error| error.to_string())?;
        }
        check_executable(&self.executable)?;
        let mut names = HashSet::new();
        let mut flags = HashSet::new();
        for argument in &self.arguments {
            if !names.insert(argument.name.as_str()) {
                return Err(format!("argument `{}` is defined twice", argument.name));
            }
            if let Binding::Option { flag } = &argument.binding
                && !flags.insert(flag.as_str())
            {
                return Err(format!("flag `{flag}` is bound twice"));
            }
            argument
                .validate()
                .map_err(|error| format!("argument `{}`: {error}", argument.name))?;
        }
        Ok(())
    }
}

impl ArgumentConfig {
    fn validate(&self) -> Result<(), String> {
        if !is_argument_name(&self.name) {
            return Err("the name must start with a letter or `_` and use letters, digits or `_` (at most 64)".to_owned());
        }
        if let Binding::Option { flag } = &self.binding
            && !is_flag(flag)
        {
            return Err(format!(
                "flag `{flag}` must be `-` or `--` followed by letters, digits, `_` or `-`"
            ));
        }
        let pattern = match &self.pattern {
            Some(pattern) => Some(
                regex::Regex::new(pattern)
                    .map_err(|error| format!("invalid `pattern`: {error}"))?,
            ),
            None => None,
        };
        if let Some(values) = &self.allowed {
            check_enum(values, pattern.as_ref())?;
        }
        Ok(())
    }
}

fn check_enum(values: &[String], pattern: Option<&regex::Regex>) -> Result<(), String> {
    if values.is_empty() {
        return Err("`enum` is empty".to_owned());
    }
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value.as_str()) {
            return Err(format!("`enum` lists `{value}` twice"));
        }
        if let Err(reason) = crate::registry::check_value_text(value) {
            return Err(format!("`enum` value `{value}`: {reason}"));
        }
        if pattern.is_some_and(|pattern| !pattern.is_match(value)) {
            return Err(format!("`enum` value `{value}` does not match `pattern`"));
        }
    }
    Ok(())
}

fn check_timeout(field: &str, seconds: u64) -> Result<(), ConfigError> {
    if (1..=TIMEOUT_LIMIT_SECONDS).contains(&seconds) {
        Ok(())
    } else {
        Err(invalid(format!(
            "`{field}` must be 1 to {TIMEOUT_LIMIT_SECONDS}"
        )))
    }
}

/// The executable must be an absolute path to a regular file (not a
/// symbolic link) that is executable and that only its owner can write.
fn check_executable(path: &Path) -> Result<(), String> {
    let shown = path.display();
    if !path.is_absolute() {
        return Err(format!("`executable` {shown} is not an absolute path"));
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("`executable` {shown}: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "`executable` {shown} is a symbolic link: give the target path"
        ));
    }
    if !metadata.is_file() {
        return Err(format!("`executable` {shown} is not a regular file"));
    }
    let mode = metadata.permissions().mode();
    if mode & 0o111 == 0 {
        return Err(format!("`executable` {shown} is not executable"));
    }
    if mode & 0o022 != 0 {
        return Err(format!(
            "`executable` {shown} is writable by its group or by others"
        ));
    }
    Ok(())
}

const fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn is_tool_name(name: &str) -> bool {
    (1..=64).contains(&name.len()) && name.chars().all(is_name_char)
}

fn is_argument_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 64
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_flag(flag: &str) -> bool {
    let body = flag
        .strip_prefix("--")
        .or_else(|| flag.strip_prefix('-'))
        .unwrap_or_default();
    body.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && body.chars().all(is_name_char)
}

fn is_env_name(name: &str) -> bool {
    is_argument_name(name)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let script = dir.path().join("script.sh");
            std::fs::write(&script, "#!/bin/sh\necho ok\n").unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self { dir }
        }

        fn script(&self) -> String {
            self.dir.path().join("script.sh").display().to_string()
        }

        fn yaml(&self, tools: &str) -> String {
            format!(
                "execution:\n  working_directory: {}\ntools:\n{tools}",
                self.dir.path().display()
            )
        }

        fn parse(&self, tools: &str) -> Result<Config, String> {
            let config: Config =
                serde_yaml::from_str(&self.yaml(tools)).map_err(|error| error.to_string())?;
            config.validate().map_err(|error| error.to_string())?;
            Ok(config)
        }

        fn tool(&self, extra: &str) -> String {
            format!(
                "  - name: deploy\n    description: Deploy\n    executable: {}\n{extra}",
                self.script()
            )
        }
    }

    #[test]
    fn a_complete_configuration_loads_with_defaults() {
        let fixture = Fixture::new();
        let tools = fixture.tool(
            "    arguments:\n      - name: env\n        type: string\n        required: true\n        enum: [staging, production]\n        binding: {kind: option, flag: --environment}\n      - name: version\n        type: string\n        pattern: '^[0-9.]+$'\n        binding: {kind: positional}\n",
        );
        let config = fixture.parse(&tools).unwrap();
        assert_eq!(config.server.bind, default_bind());
        assert_eq!(config.server.endpoint, "/mcp");
        assert_eq!(config.execution.max_concurrent, 4);
        assert_eq!(config.execution.default_timeout_seconds, 30);
        assert_eq!(config.execution.max_output_bytes, 65536);
        let arguments = &config.tools[0].arguments;
        assert_eq!(
            arguments[0].binding,
            Binding::Option {
                flag: "--environment".to_owned()
            }
        );
        assert_eq!(arguments[1].binding, Binding::Positional {});
        assert!(!arguments[1].required);
    }

    #[test]
    fn unknown_fields_are_rejected_in_every_section() {
        let fixture = Fixture::new();
        let tool = fixture.tool("");
        for yaml in [
            format!("{}\nunknown: 1\n", fixture.yaml(&tool)),
            format!("server:\n  port: 1\n{}", fixture.yaml(&tool)),
            fixture.yaml(&fixture.tool("    shell: true\n")),
            fixture.yaml(&fixture.tool(
                "    arguments:\n      - name: a\n        type: string\n        default: x\n        binding: {kind: positional}\n",
            )),
            fixture.yaml(&fixture.tool(
                "    arguments:\n      - name: a\n        type: string\n        binding: {kind: positional, flag: --a}\n",
            )),
        ] {
            let error = serde_yaml::from_str::<Config>(&yaml).expect_err(&yaml);
            assert!(error.to_string().contains("unknown field"), "{error}");
        }
    }

    #[test]
    fn unsupported_types_and_bindings_are_rejected() {
        let fixture = Fixture::new();
        for argument in [
            "type: integer\n        binding: {kind: positional}",
            "type: string\n        binding: {kind: shell}",
        ] {
            let tools = fixture.tool(&format!(
                "    arguments:\n      - name: a\n        {argument}\n"
            ));
            assert!(serde_yaml::from_str::<Config>(&fixture.yaml(&tools)).is_err());
        }
    }

    fn rejected(fixture: &Fixture, tools: &str, expected: &str) {
        let error = fixture.parse(tools).unwrap_err();
        assert!(error.contains(expected), "`{error}` lacks `{expected}`");
    }

    #[test]
    fn duplicate_and_malformed_names_are_rejected() {
        let fixture = Fixture::new();
        let tool = fixture.tool("");
        rejected(&fixture, &format!("{tool}{tool}"), "defined twice");
        rejected(
            &fixture,
            &tool.replace("deploy", "de ploy"),
            "the name must",
        );
        let argument =
            "      - name: a\n        type: string\n        binding: {kind: positional}\n";
        rejected(
            &fixture,
            &fixture.tool(&format!("    arguments:\n{argument}{argument}")),
            "argument `a` is defined twice",
        );
        rejected(
            &fixture,
            &fixture.tool("    arguments:\n      - name: 1a\n        type: string\n        binding: {kind: positional}\n"),
            "the name must start",
        );
    }

    #[test]
    fn malformed_and_conflicting_flags_are_rejected() {
        let fixture = Fixture::new();
        let option = |name: &str, flag: &str| {
            format!(
                "      - name: {name}\n        type: string\n        binding: {{kind: option, flag: '{flag}'}}\n"
            )
        };
        for flag in ["version", "---v", "--", "--a;b", "--$(id)"] {
            rejected(
                &fixture,
                &fixture.tool(&format!("    arguments:\n{}", option("a", flag))),
                "flag",
            );
        }
        rejected(
            &fixture,
            &fixture.tool(&format!(
                "    arguments:\n{}{}",
                option("a", "--x"),
                option("b", "--x")
            )),
            "bound twice",
        );
    }

    #[test]
    fn invalid_enums_and_patterns_are_rejected() {
        let fixture = Fixture::new();
        let argument = |extra: &str| {
            fixture.tool(&format!(
                "    arguments:\n      - name: a\n        type: string\n{extra}        binding: {{kind: positional}}\n"
            ))
        };
        rejected(&fixture, &argument("        enum: []\n"), "`enum` is empty");
        rejected(&fixture, &argument("        enum: [a, a]\n"), "twice");
        rejected(&fixture, &argument("        enum: ['-rf']\n"), "`-`");
        rejected(
            &fixture,
            &argument("        pattern: '('\n"),
            "invalid `pattern`",
        );
        rejected(
            &fixture,
            &argument("        enum: [abc]\n        pattern: '^[0-9]+$'\n"),
            "does not match",
        );
    }

    #[test]
    fn unsafe_limits_are_rejected() {
        let fixture = Fixture::new();
        let tool = fixture.tool("");
        for (setting, expected) in [
            ("max_concurrent: 0", "max_concurrent"),
            ("default_timeout_seconds: 0", "default_timeout_seconds"),
            ("default_timeout_seconds: 99999", "default_timeout_seconds"),
            ("max_output_bytes: 0", "max_output_bytes"),
        ] {
            let yaml = fixture
                .yaml(&tool)
                .replace("execution:\n", &format!("execution:\n  {setting}\n"));
            let config: Config = serde_yaml::from_str(&yaml).unwrap();
            let error = config.validate().unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
        }
        rejected(
            &fixture,
            &fixture.tool("    timeout_seconds: 0\n"),
            "timeout_seconds",
        );
    }

    fn rejected_executable(fixture: &Fixture, path: &Path, expected: &str) {
        let tool = fixture
            .tool("")
            .replace(&fixture.script(), &path.display().to_string());
        rejected(fixture, &tool, expected);
    }

    #[test]
    fn unsafe_executables_are_rejected() {
        let fixture = Fixture::new();
        let dir = fixture.dir.path();
        rejected_executable(&fixture, Path::new("script.sh"), "not an absolute path");
        rejected_executable(&fixture, Path::new("/no/such/script"), "No such file");
        rejected_executable(&fixture, dir, "not a regular file");
        let link = dir.join("link.sh");
        std::os::unix::fs::symlink(fixture.script(), &link).unwrap();
        rejected_executable(&fixture, &link, "symbolic link");
        let plain = dir.join("plain.sh");
        std::fs::write(&plain, "echo").unwrap();
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
        rejected_executable(&fixture, &plain, "not executable");
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o777)).unwrap();
        rejected_executable(&fixture, &plain, "writable");
    }

    #[test]
    fn a_relative_working_directory_is_rejected() {
        let fixture = Fixture::new();
        let yaml = fixture.yaml(&fixture.tool("")).replace(
            &format!("working_directory: {}", fixture.dir.path().display()),
            "working_directory: relative",
        );
        let config: Config = serde_yaml::from_str(&yaml).unwrap();
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("working_directory"), "{error}");
    }

    #[test]
    fn a_public_bind_requires_a_token() {
        let fixture = Fixture::new();
        let tool = fixture.tool("");
        let public = format!("server:\n  bind: 0.0.0.0:3000\n{}", fixture.yaml(&tool));
        let config: Config = serde_yaml::from_str(&public).unwrap();
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("auth_token_env")
        );
        let with_token = public.replace("server:\n", "server:\n  auth_token_env: RUNNER_TOKEN\n");
        let config: Config = serde_yaml::from_str(&with_token).unwrap();
        config.validate().unwrap();
    }

    #[test]
    fn the_endpoint_must_be_a_plain_path() {
        let fixture = Fixture::new();
        let tool = fixture.tool("");
        for endpoint in ["mcp", "/", "/health", "/a//b", "/a?b"] {
            let yaml = format!("server:\n  endpoint: '{endpoint}'\n{}", fixture.yaml(&tool));
            let config: Config = serde_yaml::from_str(&yaml).unwrap();
            assert!(config.validate().is_err(), "{endpoint}");
        }
    }

    #[test]
    fn missing_and_invalid_files_are_reported() {
        let error = Config::load(Path::new("/no/such/config.yaml")).unwrap_err();
        assert!(matches!(error, ConfigError::Read { .. }));
        let fixture = Fixture::new();
        let path = fixture.dir.path().join("config.yaml");
        std::fs::write(&path, "tools: [").unwrap();
        assert!(matches!(
            Config::load(&path).unwrap_err(),
            ConfigError::Parse { .. }
        ));
        std::fs::write(&path, fixture.yaml(&fixture.tool(""))).unwrap();
        assert_eq!(Config::load(&path).unwrap().tools[0].name, "deploy");
    }
}
