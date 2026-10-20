//! The worker's init duties as PID 1 (`saci_worker::init`).
//!
//! The test process makes itself a child subreaper, so orphans come to it
//! as they come to PID 1 in a container. It runs without the test harness:
//! the init blocks signals and reaps every child of the process.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[cfg(target_os = "linux")]
fn main() {
    use std::process::Command;

    use saci_worker::init::supervise;

    nix::sys::prctl::set_child_subreaper(true).unwrap();

    // Without the init, the orphan stays a zombie of the subreaper.
    let status = Command::new("bash")
        .args(["-c", ORPHAN_ZOMBIES])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(1), "the check must see the zombie");

    // With the init, the orphan is reaped as soon as it ends.
    let mut orphan = Command::new("bash");
    orphan.args(["-c", ORPHAN_ZOMBIES]);
    assert_eq!(supervise(orphan).unwrap(), 0, "the orphan was not reaped");

    // Stop signals reach the child, and its exit code is returned.
    let mut stopped = Command::new("bash");
    stopped.args([
        "-c",
        "trap 'exit 7' TERM; sleep 0.2; kill -TERM $PPID; while :; do sleep 0.05; done",
    ]);
    assert_eq!(supervise(stopped).unwrap(), 7);

    // A child ended by a signal gives 128 plus the signal number.
    let mut killed = Command::new("bash");
    killed.args(["-c", "kill -KILL $$"]);
    assert_eq!(supervise(killed).unwrap(), 128 + 9);

    println!("worker_init: ok");
}

/// Orphans a short `sleep` (its subshell exits at once), lets it end, then
/// exits 1 if a zombie whose parent is this script's parent remains.
#[cfg(target_os = "linux")]
const ORPHAN_ZOMBIES: &str = r#"
( sleep 0.1 & )
sleep 1
for stat in /proc/[0-9]*/stat; do
    read -r _ comm state ppid _ < "$stat" 2>/dev/null || continue
    if [ "$state" = Z ] && [ "$ppid" = "$PPID" ]; then
        echo "zombie: $stat $comm"
        exit 1
    fi
done
exit 0
"#;

#[cfg(not(target_os = "linux"))]
fn main() {
    // Child subreapers exist on Linux only; containers run Linux.
}
