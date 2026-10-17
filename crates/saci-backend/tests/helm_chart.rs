//! The Helm chart in `deploy/helm/saci` renders a valid Saci configuration
//! and the namespace-scoped, single-writer, non-root install it promises.
//! Needs `helm` on `PATH` (CI has it); skipped otherwise.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::Command;

use saci_core::config::{Config, Runtime};
use serde_yaml::Value;

/// The documents `helm template` renders with `args`, or `None` without helm.
fn render(args: &[&str]) -> Option<Vec<Value>> {
    let chart = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/helm/saci");
    let output = Command::new("helm")
        .args(["template", "demo"])
        .arg(&chart)
        .args(["--namespace", "tests"])
        .args(args)
        .output();
    let Ok(output) = output else {
        eprintln!("helm is not on PATH; skipping the chart checks");
        return None;
    };
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let documents = text
        .split("\n---")
        .filter_map(|document| serde_yaml::from_str::<Value>(document).ok())
        .filter(|document| document.get("kind").is_some())
        .collect();
    Some(documents)
}

fn find<'a>(documents: &'a [Value], kind: &str) -> &'a Value {
    documents
        .iter()
        .find(|document| document["kind"].as_str() == Some(kind))
        .unwrap_or_else(|| panic!("no {kind} rendered"))
}

fn config(documents: &[Value]) -> Config {
    let text = find(documents, "ConfigMap")["data"]["saci.yaml"]
        .as_str()
        .unwrap();
    Config::from_yaml(text, "saci.yaml").unwrap_or_else(|error| panic!("{error}\n{text}"))
}

#[test]
fn the_default_install_is_valid_and_stays_in_its_namespace() {
    let Some(documents) = render(&[]) else { return };
    let config = config(&documents);
    assert_eq!(config.worker.runtime, Runtime::Kubernetes);
    assert_eq!(config.worker.kubernetes.namespace.as_deref(), Some("tests"));
    assert_eq!(
        config.worker.kubernetes.service_account.as_deref(),
        Some("demo-saci-worker")
    );
    assert_eq!(
        config.backend.worker_url.as_deref(),
        Some("http://demo-saci.tests.svc:8080")
    );
    assert!(config.backend.auth.is_some());
}

#[test]
fn nothing_cluster_wide_is_installed() {
    let Some(documents) = render(&[]) else { return };
    let kinds: Vec<&str> = documents
        .iter()
        .filter_map(|d| d["kind"].as_str())
        .collect();
    for cluster_wide in [
        "ClusterRole",
        "ClusterRoleBinding",
        "Namespace",
        "CustomResourceDefinition",
    ] {
        assert!(!kinds.contains(&cluster_wide), "{kinds:?}");
    }
    let rules = find(&documents, "Role")["rules"].as_sequence().unwrap();
    assert!(rules.iter().all(|rule| {
        !rule["verbs"]
            .as_sequence()
            .unwrap()
            .iter()
            .any(|v| v == "*")
    }));
}

#[test]
fn one_non_root_backend_writes_the_database() {
    let Some(documents) = render(&[]) else { return };
    let deployment = find(&documents, "Deployment");
    assert_eq!(deployment["spec"]["replicas"], 1);
    assert_eq!(deployment["spec"]["strategy"]["type"], "Recreate");
    let pod = &deployment["spec"]["template"]["spec"];
    let container = &pod["containers"][0];
    assert_eq!(container["securityContext"]["runAsNonRoot"], true);
    assert_eq!(
        container["securityContext"]["allowPrivilegeEscalation"],
        false
    );
    assert!(container["securityContext"]["runAsUser"].is_null());
    assert_eq!(pod["securityContext"]["fsGroup"], 10001);
    let claim = &pod["volumes"][1]["persistentVolumeClaim"]["claimName"];
    assert_eq!(*claim, "demo-saci-data");
    assert_eq!(
        find(&documents, "PersistentVolumeClaim")["spec"]["resources"]["requests"]["storage"],
        "1Gi"
    );
}

#[test]
fn openshift_assigns_ids_and_existing_claims_are_used() {
    let args = [
        "--api-versions",
        "security.openshift.io/v1",
        "--set",
        "persistence.existingClaim=saci-db",
    ];
    let Some(documents) = render(&args) else {
        return;
    };
    let pod = &find(&documents, "Deployment")["spec"]["template"]["spec"];
    assert!(
        pod["securityContext"].is_null(),
        "OpenShift chooses the UID and fsGroup"
    );
    assert_eq!(
        pod["volumes"][1]["persistentVolumeClaim"]["claimName"],
        "saci-db"
    );
    assert!(
        !documents
            .iter()
            .any(|d| d["kind"] == "PersistentVolumeClaim")
    );
}

#[test]
fn values_reach_the_configuration() {
    let args = [
        "--set",
        "llm.apiKey.existingSecret=saci-llm",
        "--set",
        "auth.existingSecret=saci-auth",
        "--set",
        "config.environments.staging.parameters.SERVICE_URL=http://demo:8080",
        "--set",
        "config.environments.staging.secrets.TOKEN.secret=demo",
        "--set",
        "config.environments.staging.secrets.TOKEN.key=token",
        "--set",
        "config.limits.run_timeout_secs=600",
    ];
    let Some(documents) = render(&args) else {
        return;
    };
    let config = config(&documents);
    let key = config.llm.api_key_secret.as_ref().unwrap();
    assert_eq!(
        (key.name.as_str(), key.key.as_str()),
        ("saci-llm", "api-key")
    );
    assert!(config.environment("staging").is_ok());
    assert!(
        !documents.iter().any(|d| d["kind"] == "Secret"),
        "existing Secrets are used"
    );
}
