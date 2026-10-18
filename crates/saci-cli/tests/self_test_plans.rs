//! Saci's own test plans (`test/plans`, `test/README.md`) stay valid:
//! every plan bundles and expands, and the configuration that runs them
//! loads. Running them needs a real model and is done by hand.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use saci_core::bundle;
use saci_core::config::Config;

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

#[test]
fn the_self_test_configuration_loads() {
    let path = repository().join("test/saci.yaml");
    let config = Config::load(&path).unwrap_or_else(|error| panic!("{error}"));
    let (environment, _) = config.environment("saci-under-test").unwrap();
    assert_eq!(
        environment.parameters["UNDER_TEST_URL"],
        "http://127.0.0.1:18080"
    );
    assert_eq!(environment.parameters["UNDER_TEST_ENVIRONMENT"], "local");
    assert_eq!(config.worker.network.as_deref(), Some("host"));
}
