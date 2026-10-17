//! The `bash` process behind a shell session.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::process::{ExitStatus, Stdio};

use nix::sys::signal::{Signal, kill, killpg};
use nix::unistd::Pid;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

/// Chunks read from one pipe; closed at end of file.
pub type Pipe = mpsc::Receiver<Vec<u8>>;

/// A running `bash` in its own process group.
#[derive(Debug)]
pub struct Process {
    child: Child,
    stdin: ChildStdin,
    /// Standard output chunks.
    pub stdout: Pipe,
    /// Standard error chunks.
    pub stderr: Pipe,
    pid: Pid,
}

impl Process {
    /// Starts `bash` with exactly `environment`, in `workspace`.
    ///
    /// # Errors
    ///
    /// Fails when `bash` cannot be started.
    pub fn spawn(
        workspace: &Path,
        environment: &BTreeMap<String, String>,
    ) -> std::io::Result<Self> {
        let mut child = Command::new("bash")
            .args(["--noprofile", "--norc"])
            .env_clear()
            .envs(environment)
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()?;
        let missing = || std::io::Error::other("bash pipes were not created");
        let stdin = child.stdin.take().ok_or_else(missing)?;
        let stdout = pump(child.stdout.take().ok_or_else(missing)?);
        let stderr = pump(child.stderr.take().ok_or_else(missing)?);
        let raw = child.id().and_then(|id| i32::try_from(id).ok());
        let pid = Pid::from_raw(raw.ok_or_else(|| std::io::Error::other("bash has no pid"))?);
        Ok(Self {
            child,
            stdin,
            stdout,
            stderr,
            pid,
        })
    }

    /// Sends one line of input to `bash`.
    ///
    /// # Errors
    ///
    /// Fails when the shell no longer reads its input.
    pub async fn send(&mut self, line: &str) -> std::io::Result<()> {
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await
    }

    /// Discards output produced between commands, for example by background
    /// jobs.
    pub fn drain(&mut self) {
        while self.stdout.try_recv().is_ok() {}
        while self.stderr.try_recv().is_ok() {}
    }

    /// Kills every descendant of `bash`, leaving `bash` itself running.
    pub fn kill_descendants(&self) {
        for pid in descendants(self.pid) {
            let _ = kill(pid, Signal::SIGKILL);
        }
    }

    /// Kills `bash` and its process group and waits for it.
    pub async fn kill(&mut self) -> Option<ExitStatus> {
        self.kill_descendants();
        let _ = killpg(self.pid, Signal::SIGKILL);
        self.child.wait().await.ok()
    }

    /// The exit status, if `bash` has exited.
    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    /// Waits for `bash` to exit.
    pub async fn wait(&mut self) -> Option<ExitStatus> {
        self.child.wait().await.ok()
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.kill_descendants();
        let _ = killpg(self.pid, Signal::SIGKILL);
    }
}

fn pump(mut reader: impl AsyncRead + Unpin + Send + 'static) -> Pipe {
    let (sender, receiver) = mpsc::channel(64);
    tokio::spawn(async move {
        let mut buffer = vec![0_u8; 8192];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if sender.send(buffer[..read].to_vec()).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
    receiver
}

/// Every live process descended from `root`, found through `/proc`.
fn descendants(root: Pid) -> Vec<Pid> {
    let mut children: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<i32>().ok())
        else {
            continue;
        };
        if let Some(parent) = parent_of(pid) {
            children.entry(parent).or_default().push(pid);
        }
    }
    let mut found = Vec::new();
    let mut queue = VecDeque::from([root.as_raw()]);
    while let Some(pid) = queue.pop_front() {
        for child in children.get(&pid).into_iter().flatten() {
            found.push(Pid::from_raw(*child));
            queue.push_back(*child);
        }
    }
    found
}

fn parent_of(pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is parenthesized and may contain spaces.
    let after = stat.rsplit_once(')')?.1;
    after.split_whitespace().nth(1)?.parse().ok()
}
