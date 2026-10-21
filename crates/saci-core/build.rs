//! Records the commit the binaries are built from (docs/development.md#build-commit)
//! as `SACI_BUILD_COMMIT`: the value of the `SACI_BUILD_COMMIT` environment
//! variable when it is set, as in container builds without `.git`, else the
//! `HEAD` commit that `git` reports. Without either the commit is unknown.

use std::env;
use std::process::Command;

const COMMIT: &str = "SACI_BUILD_COMMIT";

/// The trimmed output of `git <args>`, run in this crate's directory.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (output.status.success() && !text.is_empty()).then(|| text.to_owned())
}

fn main() {
    println!("cargo::rerun-if-env-changed={COMMIT}");
    if let Some(commit) = env::var(COMMIT)
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        println!("cargo::rustc-env={COMMIT}={}", commit.trim());
        return;
    }
    let Some(commit) = git(&["rev-parse", "HEAD"]) else {
        return;
    };
    // A new commit or a branch switch changes these files, so Cargo then
    // records the new commit.
    let mut watched = vec!["HEAD".to_owned(), "packed-refs".to_owned()];
    watched.extend(git(&["symbolic-ref", "-q", "HEAD"]));
    for name in watched {
        if let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", &name]) {
            println!("cargo::rerun-if-changed={path}");
        }
    }
    println!("cargo::rustc-env={COMMIT}={commit}");
}
