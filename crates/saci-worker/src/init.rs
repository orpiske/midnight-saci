//! Init duties when the worker is PID 1 in its container.
//!
//! A container runtime starts the image's entry point as PID 1. The kernel
//! gives every orphaned process to PID 1, and only its parent can reap a
//! process that has ended. Closing the run's shell kills `bash` and what it
//! started, which orphans them; a background job whose parent exited is an
//! orphan too. A worker that is PID 1 would keep all of them as zombies, so
//! it starts itself again as a child and does the init work: it forwards
//! stop signals to the child, reaps every process that ends, and exits with
//! the child's code.

use std::process::Command;

use nix::sys::signal::{SigSet, Signal, kill};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::Pid;

/// Signals passed on to the child; the rest stay with the init.
const FORWARDED: [Signal; 4] = [
    Signal::SIGTERM,
    Signal::SIGINT,
    Signal::SIGHUP,
    Signal::SIGQUIT,
];

/// Whether this process is PID 1, the init of its PID namespace.
#[must_use]
pub fn is_pid_one() -> bool {
    nix::unistd::getpid().as_raw() == 1
}

/// Starts `command`, forwards [`FORWARDED`] signals to it and reaps every
/// child that ends, its own and adopted ones, until `command` exits.
/// Returns its exit code, or 128 plus the signal number if a signal ended
/// it. Call it before any other thread starts: it blocks the signals it
/// waits for on the calling thread.
///
/// # Errors
///
/// Fails when `command` cannot be started or the signals cannot be blocked.
pub fn supervise(mut command: Command) -> std::io::Result<u8> {
    // Spawned first: a child inherits the signal mask, and the worker must
    // not start with its stop signals blocked.
    let child = command.spawn()?;
    let pid = i32::try_from(child.id())
        .map(Pid::from_raw)
        .map_err(std::io::Error::other)?;
    // The child is reaped through `waitpid` below, with the orphans.
    drop(child);
    let mut signals = SigSet::empty();
    for signal in FORWARDED {
        signals.add(signal);
    }
    signals.add(Signal::SIGCHLD);
    // Blocked signals stay pending, even for PID 1, until `wait` takes
    // them.
    signals.thread_block()?;
    // A child that ended before the block sent its SIGCHLD too early.
    let mut ended = reap(pid);
    loop {
        if let Some(code) = ended {
            signals.thread_unblock()?;
            return Ok(code);
        }
        match signals.wait()? {
            Signal::SIGCHLD => ended = reap(pid),
            signal => {
                let _ = kill(pid, signal);
            }
        }
    }
}

/// Reaps every child that has ended; returns the exit code of `child` if
/// it is one of them.
fn reap(child: Pid) -> Option<u8> {
    let mut code = None;
    loop {
        match waitpid(None, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(pid, status)) if pid == child => {
                code = Some(u8::try_from(status).unwrap_or(u8::MAX));
            }
            Ok(WaitStatus::Signaled(pid, signal, _)) if pid == child => {
                code = Some(u8::try_from(128 + signal as i32).unwrap_or(u8::MAX));
            }
            Ok(WaitStatus::StillAlive) | Err(_) => return code,
            Ok(_) => {}
        }
    }
}
