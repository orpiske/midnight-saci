//! Explicit credential projection.
//!
//! The worker process holds two kinds of credentials: the LLM transport key
//! (under `llm.api_key_env`) and the target environment's secrets (each under
//! `SACI_SECRET_<NAME>`, set by the launcher). Only the target secrets, the
//! environment's parameters and a short list of ordinary variables reach the
//! shell, and only parameters and secrets can be expanded into HTTP requests.
//! The LLM key reaches neither. Every known credential is registered with the
//! redactor so accidental disclosure in output is masked.

use std::collections::BTreeMap;

use saci_core::redaction::Redactor;
use saci_core::run::{ErrorKind, ExecutionError};
use saci_core::spec::{EnvironmentSpec, LlmSettings};

/// Prefix of the worker variables carrying target secret values.
pub const SECRET_PREFIX: &str = "SACI_SECRET_";

/// Worker variable carrying the run-scoped backend token.
pub const WORKER_TOKEN_ENV: &str = "SACI_WORKER_TOKEN";

/// Ordinary variables the shell inherits from the worker.
const INHERITED: [&str; 19] = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "TMPDIR",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
];

/// The credentials and variables a run may use, already separated.
#[derive(Debug, Clone)]
pub struct Credentials {
    parameters: BTreeMap<String, String>,
    secrets: BTreeMap<String, String>,
    shell: BTreeMap<String, String>,
    redactor: Redactor,
}

impl Credentials {
    /// Separates the worker's environment into what tools may use.
    ///
    /// # Errors
    ///
    /// Fails when a configured secret was not supplied to the worker.
    pub fn project(
        environment: &EnvironmentSpec,
        llm: &LlmSettings,
        process: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, ExecutionError> {
        let process: BTreeMap<String, String> = process.into_iter().collect();
        let mut redactor = Redactor::new();
        let llm_key = llm.api_key_env.as_deref();
        if let Some(key) = llm_key.and_then(|name| process.get(name)) {
            redactor.add("LLM_API_KEY", key);
        }
        if let Some(token) = process.get(WORKER_TOKEN_ENV) {
            redactor.add("WORKER_TOKEN", token);
        }
        let secrets = secret_values(environment, &process, &mut redactor)?;
        let parameters = environment.parameters.clone();
        let mut shell: BTreeMap<String, String> = INHERITED
            .iter()
            .filter(|name| Some(**name) != llm_key)
            .filter_map(|name| Some(((*name).to_owned(), process.get(*name)?.clone())))
            .collect();
        shell.extend(parameters.clone());
        shell.extend(secrets.clone());
        Ok(Self {
            parameters,
            secrets,
            shell,
            redactor,
        })
    }

    /// The complete environment of the shell session.
    #[must_use]
    pub const fn shell_environment(&self) -> &BTreeMap<String, String> {
        &self.shell
    }

    /// Redacts every known credential.
    #[must_use]
    pub const fn redactor(&self) -> &Redactor {
        &self.redactor
    }

    /// What the model is told about the environment: parameter values and
    /// secret names, never secret values.
    #[must_use]
    pub fn model_context(&self) -> String {
        let mut text = String::new();
        if !self.parameters.is_empty() {
            text.push_str("Environment parameters (also exported to the shell):\n");
            for (name, value) in &self.parameters {
                text.push_str(&format!("- {name} = {value}\n"));
            }
        }
        if !self.secrets.is_empty() {
            text.push_str(
                "Secrets (values hidden; use $NAME in shell commands or ${NAME} in HTTP requests):\n",
            );
            for name in self.secrets.keys() {
                text.push_str(&format!("- {name}\n"));
            }
        }
        text
    }

    /// Replaces `${NAME}` with the value of a parameter or secret. Other
    /// text, including `${…}` naming anything else, is left as written.
    #[must_use]
    pub fn expand(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("${") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let value = after
                .find('}')
                .and_then(|end| Some((self.lookup(&after[..end])?, end)));
            if let Some((value, end)) = value {
                out.push_str(value);
                rest = &after[end + 1..];
            } else {
                out.push_str("${");
                rest = after;
            }
        }
        out.push_str(rest);
        out
    }

    fn lookup(&self, name: &str) -> Option<&str> {
        self.secrets
            .get(name)
            .or_else(|| self.parameters.get(name))
            .map(String::as_str)
    }
}

fn secret_values(
    environment: &EnvironmentSpec,
    process: &BTreeMap<String, String>,
    redactor: &mut Redactor,
) -> Result<BTreeMap<String, String>, ExecutionError> {
    let mut secrets = BTreeMap::new();
    for name in environment.secrets.keys() {
        let Some(value) = process.get(&format!("{SECRET_PREFIX}{name}")) else {
            return Err(ExecutionError::new(
                ErrorKind::InvalidSpecification,
                format!(
                    "secret `{name}` of environment `{}` was not supplied to the worker",
                    environment.name
                ),
            ));
        };
        if !redactor.add(name, value) {
            tracing::warn!(secret = %name, "secret is too short to be redacted from evidence");
        }
        secrets.insert(name.clone(), value.clone());
    }
    Ok(secrets)
}

#[cfg(test)]
mod tests {
    use saci_core::spec::SecretRef;

    use super::*;

    fn environment() -> EnvironmentSpec {
        EnvironmentSpec {
            name: "local".into(),
            parameters: BTreeMap::from([("BASE_URL".into(), "http://target".into())]),
            secrets: BTreeMap::from([(
                "TARGET_TOKEN".into(),
                SecretRef::Env {
                    env: "BACKEND_TARGET_TOKEN".into(),
                },
            )]),
            ..EnvironmentSpec::default()
        }
    }

    fn llm() -> LlmSettings {
        LlmSettings {
            base_url: "http://model/v1".into(),
            model: "m".into(),
            api_key_env: Some("SACI_LLM_API_KEY".into()),
            api_key_secret: None,
        }
    }

    fn process() -> Vec<(String, String)> {
        [
            ("PATH", "/usr/bin"),
            ("SACI_LLM_API_KEY", "sk-llm-key"),
            ("SACI_SECRET_TARGET_TOKEN", "target-token-value"),
            (WORKER_TOKEN_ENV, "worker-token-value"),
            ("UNRELATED", "leak"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
    }

    #[test]
    fn the_shell_gets_parameters_and_target_secrets_only() {
        let credentials = Credentials::project(&environment(), &llm(), process()).unwrap();
        let shell = credentials.shell_environment();
        assert_eq!(
            shell.keys().collect::<Vec<_>>(),
            ["BASE_URL", "PATH", "TARGET_TOKEN"]
        );
        assert_eq!(shell["TARGET_TOKEN"], "target-token-value");
        assert!(!shell.values().any(|v| v.contains("sk-llm-key")));
    }

    #[test]
    fn every_known_credential_is_redacted() {
        let credentials = Credentials::project(&environment(), &llm(), process()).unwrap();
        let text = "sk-llm-key target-token-value worker-token-value";
        assert_eq!(
            credentials.redactor().redact(text),
            "[REDACTED:LLM_API_KEY] [REDACTED:TARGET_TOKEN] [REDACTED:WORKER_TOKEN]"
        );
    }

    #[test]
    fn expansion_covers_parameters_and_secrets_but_not_the_llm_key() {
        let credentials = Credentials::project(&environment(), &llm(), process()).unwrap();
        assert_eq!(
            credentials.expand("${BASE_URL}/x Bearer ${TARGET_TOKEN} ${SACI_LLM_API_KEY} ${open"),
            "http://target/x Bearer target-token-value ${SACI_LLM_API_KEY} ${open"
        );
        let context = credentials.model_context();
        assert!(context.contains("BASE_URL = http://target"));
        assert!(context.contains("- TARGET_TOKEN"));
        assert!(!context.contains("target-token-value"));
    }

    #[test]
    fn missing_secrets_are_errors() {
        let error = Credentials::project(&environment(), &llm(), Vec::new()).unwrap_err();
        assert!(error.message.contains("TARGET_TOKEN"), "{}", error.message);
    }
}
