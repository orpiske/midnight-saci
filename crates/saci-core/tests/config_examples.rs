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
