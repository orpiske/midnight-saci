#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The example configurations in `examples/script-runner` are valid, and the
//! email example sends the expected message.
//!
//! The examples use installation paths, so each test copies the scripts to a
//! temporary directory and points the configuration there.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use saci_script_runner::config::Config;
use saci_script_runner::executor::{self, Execution, Status};
use saci_script_runner::registry::Registry;
use serde_json::json;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/script-runner")
}

/// Copies `files` (relative to the examples directory) below `root` with
/// mode 0755, and returns the configuration `name` with its installation
/// paths replaced by `root`.
fn install(root: &Path, name: &str, files: &[&str]) -> Config {
    for file in files {
        let target = root.join(file);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(examples().join(file), &target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let root = root.display().to_string();
    let text = std::fs::read_to_string(examples().join(name))
        .unwrap()
        .replace("/opt/saci-script-runner", &root)
        .replace("/var/lib/saci-mailer", &root);
    let config: Config = serde_yaml::from_str(&text).unwrap();
    config
        .validate()
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    config
}

#[test]
fn the_basic_example_is_valid() {
    let root = tempfile::tempdir().unwrap();
    let scripts = [
        "scripts/health.sh",
        "scripts/disk-usage.sh",
        "scripts/greet.sh",
    ];
    let config = install(root.path(), "config.yaml", &scripts);
    let names: Vec<&str> = config.tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, ["health_check", "disk_usage", "greet"]);
}

/// Puts a stand-in for `mutt` first on the script's `PATH`. It writes its
/// arguments and the message to the returned file.
fn fake_mutt(root: &Path, config: &mut Config) -> PathBuf {
    let bin = root.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let mail = root.join("mail.txt");
    let mutt = bin.join("mutt");
    let stub = format!(
        "#!/bin/sh\n{{ echo \"$*\"; cat; }} > '{}'\n",
        mail.display()
    );
    std::fs::write(&mutt, stub).unwrap();
    std::fs::set_permissions(&mutt, std::fs::Permissions::from_mode(0o755)).unwrap();
    let environment = &mut config.execution.environment;
    let path = format!("{}:{}", bin.display(), environment["PATH"]);
    environment.insert("PATH".to_owned(), path);
    mail
}

async fn call(config: &Config, name: &str, input: &serde_json::Value) -> Execution {
    let registry = Registry::new(config).unwrap();
    let tool = registry.get(name).unwrap();
    let arguments = tool.arguments(input.as_object()).unwrap();
    let request = executor::Request {
        executable: &tool.executable,
        arguments: &arguments,
        working_directory: &config.execution.working_directory,
        environment: &config.execution.environment,
        timeout: tool.timeout,
        max_output_bytes: config.execution.max_output_bytes,
    };
    executor::run(&request, std::future::pending()).await
}

#[tokio::test]
async fn the_email_example_sends_the_run_summary() {
    let root = tempfile::tempdir().unwrap();
    let mut config = install(root.path(), "email/runner.yaml", &["email/send-email.sh"]);
    assert!(config.server.auth.is_some());
    let mail = fake_mutt(root.path(), &mut config);
    let input = json!({
        "run_id": "run-0f3c9a2b7d4e4f1a9b8c7d6e5f4a3b2c",
        "title": "Service health",
        "outcome": "failed",
        "reason": "",
        "finished_at": "2026-10-22T10:03:10Z",
        "counts": "3 passed, 1 failed, 1 skipped",
        "summary": "Units:\n- [failed] 1.2 Check orders: 500 returned",
    });
    let result = call(&config, "send_report_email", &input).await;
    assert_eq!(result.status, Status::Completed, "{}", result.stderr);

    let mail = std::fs::read_to_string(mail).unwrap();
    assert_eq!(
        mail.lines().next().unwrap(),
        "-s Saci Report: Service health - failed - 2026-10-22 -- qa-team@example.com"
    );
    assert!(mail.contains("Units run:    5\n"), "{mail}");
    assert!(
        mail.contains("- [failed] 1.2 Check orders: 500 returned"),
        "{mail}"
    );
}
