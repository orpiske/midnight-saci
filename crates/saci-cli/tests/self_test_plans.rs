//! Saci's own test plans (`test/plans`, `test/README.md`) stay valid:
//! every plan bundles and expands, and the configuration that runs them
//! loads. Running them needs a real model and is done by hand.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use saci_core::bundle;
use saci_core::config::Config;
use saci_core::spec::SecretRef;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every plan in `test/plans` and its number of units. Headings inside
/// the plans' heredocs belong to the plans they write, not to these.
const PLANS: [(&str, usize); 3] = [
    ("cli-authoring.md", 14),
    ("backend-api.md", 19),
    ("cli-runs.md", 14),
];

#[test]
fn every_self_test_plan_is_valid() {
    let directory = repository().join("test/plans");
    let mut found: Vec<String> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".md"))
        .collect();
    found.sort_unstable();
    let mut listed: Vec<String> = PLANS.iter().map(|(name, _)| (*name).to_owned()).collect();
    listed.sort_unstable();
    assert_eq!(found, listed, "list every plan in test/plans here");

    for (name, units) in PLANS {
        let path = directory.join(name);
        let loaded = bundle::load(&path).unwrap_or_else(|error| panic!("{name}: {error}"));
        let expanded = bundle::expand(&loaded).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(expanded.units.len(), units, "{name}");
    }
}

/// The environment parameters the plans read. The values depend on where
/// the two Saci instances run, so only their presence is checked.
const PARAMETERS: [&str; 3] = ["UNDER_TEST_URL", "UNDER_TEST_ENVIRONMENT", "SERVICE_URL"];

#[test]
fn the_self_test_configuration_loads() {
    let path = repository().join("test/saci.yaml");
    let config = Config::load(&path).unwrap_or_else(|error| panic!("{error}"));
    let (environment, _) = config.environment("saci-under-test").unwrap();
    for name in PARAMETERS {
        let value = environment
            .parameters
            .get(name)
            .unwrap_or_else(|| panic!("saci-under-test does not define {name}"));
        assert!(!value.is_empty(), "{name} is empty");
    }
    // The plans test this same backend, so they get its own client token.
    let auth = config.backend.auth.as_ref().expect("backend.auth is set");
    assert_eq!(
        environment.secrets.get("UNDER_TEST_TOKEN"),
        Some(&SecretRef::Env {
            env: auth.token_env.clone()
        }),
        "UNDER_TEST_TOKEN must come from backend.auth.token_env"
    );
    assert_eq!(config.worker.network.as_deref(), Some("host"));
}
