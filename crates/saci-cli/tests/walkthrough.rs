//! The walkthrough's files (`examples/walkthrough`, `docs/walkthrough.md`)
//! stay valid: the configuration loads, every plan parses, and the
//! scripted model has an entry for every unit the walkthrough runs. The
//! walkthrough itself runs in CI's `worker-image` job.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use saci_core::bundle;
use saci_core::config::Config;
use saci_testkit::playbook::Playbook;
use serde_json::json;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The plans the walkthrough runs: the starter template (what
/// `saci new test-plan` writes) and the example plans.
fn plans() -> Vec<PathBuf> {
    let root = repository();
    vec![
        root.join("docs/templates/test-plan.md"),
        root.join("examples/walkthrough/plans/orders.md"),
        root.join("examples/walkthrough/plans/export.md"),
    ]
}

#[test]
fn the_walkthrough_configuration_loads() {
    let path = repository().join("examples/walkthrough/saci.yaml");
    let config = Config::from_yaml(&std::fs::read_to_string(&path).unwrap(), "saci.yaml").unwrap();
    let (environment, _) = config.environment("local").unwrap();
    assert!(environment.parameters.contains_key("SERVICE_URL"));
    assert_eq!(config.worker.network.as_deref(), Some("host"));
}

#[test]
fn the_playbook_covers_every_walkthrough_unit() {
    let path = repository().join("examples/walkthrough/playbook.json");
    let book = Playbook::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    for plan in plans() {
        let expanded = bundle::expand(&bundle::load(&plan).unwrap()).unwrap();
        for unit in &expanded.units {
            let title = &unit.identity.title;
            let prompt =
                format!("Current unit: Test 1 \"{title}\" (phase \"P\", plan.md:1), attempt 1.");
            let request = json!({"messages": [{"role": "user", "content": prompt}]});
            let reply = book
                .reply(&request)
                .unwrap_or_else(|error| panic!("{}: {error}", plan.display()));
            let calls = &reply["choices"][0]["message"]["tool_calls"];
            assert!(calls.is_array(), "{title}: {reply}");
        }
    }
}
