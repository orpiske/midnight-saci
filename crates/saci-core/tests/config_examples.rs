//! Every example configuration shipped in `examples/config` is valid.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use saci_core::config::{Config, Runtime};

fn load(name: &str) -> Config {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/config")
        .join(name);
    Config::load(&path).unwrap_or_else(|error| panic!("{error}"))
}

#[test]
fn local_example_is_valid() {
    let config = load("local.yaml");
    assert_eq!(config.worker.runtime, Runtime::Auto);
    let (environment, limits) = config.environment("local").unwrap();
    assert!(environment.secrets.contains_key("SERVICE_TOKEN"));
    assert_eq!(limits.step_timeout_secs, 300);
    assert_eq!(config.policy("retry-errors").unwrap().retry.max_attempts, 2);
}

#[test]
fn kubernetes_example_is_valid() {
    let config = load("kubernetes.yaml");
    assert_eq!(config.worker.runtime, Runtime::Kubernetes);
    assert!(config.backend.auth.is_some());
    assert!(config.environment("same-namespace").is_ok());
}

/// The outcome action of the script runner email example
/// (`examples/script-runner/email`) is valid in a backend configuration.
#[test]
fn script_runner_email_action_is_valid() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let read = |path: &str| std::fs::read_to_string(examples.join(path)).unwrap();
    let base = read("config/local.yaml").replace(
        "policies:\n",
        "policies:\n  email:\n    actions: [email-report]\n",
    );
    let text = format!(
        "{base}\n{}",
        read("script-runner/email/backend-actions.yaml")
    );
    let config =
        Config::from_yaml(&text, "email example").unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(config.policies["email"].actions, ["email-report"]);
    let action = &config.actions.definitions["email-report"];
    assert_eq!(action.tool, "send_report_email");
    assert_eq!(action.arguments["summary"], "Units:\n{{summary}}");
}
