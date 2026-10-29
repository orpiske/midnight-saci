//! `saci watch`: polls Git repositories and runs test plans when they
//! change (`docs/watch.md`).

mod backend;
mod config;
mod git;
mod state;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use reqwest::Method;
use saci_core::api::{IDEMPOTENCY_KEY, RunSummary, SubmitRun};
use saci_core::bundle;
use saci_core::ids::RunId;
use saci_core::plan::PlanBundle;
use saci_core::status::{Lifecycle, Outcome};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::watch;

use crate::client::{CliError, Client, Connection, exit};
use backend::LocalBackend;
use config::{PlanEntry, Trigger, WatchConfig};
use state::{Cycle, LastCycle, PlanRun, Snapshot, State, snapshot_hash};

/// Seconds between two submissions of a plan that the backend could not take.
const RETRY_SECS: u64 = 10;

/// Arguments of `saci watch`.
#[derive(Debug, Clone, clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct WatchArgs {
    #[command(subcommand)]
    pub command: Option<WatchCommand>,
    /// Configuration file (docs/watch.md).
    #[arg(long, short, default_value = "watch.yaml")]
    pub config: PathBuf,
    /// Poll one time, run the cycles that are due, then exit.
    #[arg(long)]
    pub once: bool,
    /// Start a cycle of every trigger now, also without a change.
    #[arg(long)]
    pub run_now: bool,
}

/// Subcommands of `saci watch`.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WatchCommand {
    /// Show the last commits seen, the last cycle of each trigger and the
    /// run the watcher waits for. Works offline.
    Status {
        /// Configuration file (docs/watch.md).
        #[arg(long, short, default_value = "watch.yaml")]
        config: PathBuf,
    },
}

/// Runs the watcher until SIGINT or SIGTERM, or with `--once` until the
/// due cycles ended.
///
/// # Errors
///
/// A [`CliError`] for an invalid configuration or state file, a first
/// start that cannot read a branch, or a state file that cannot be written.
pub async fn run(connection: &Connection, args: &WatchArgs) -> Result<u8, CliError> {
    let config = WatchConfig::load(&args.config)?;
    std::fs::create_dir_all(config.repos_dir()).map_err(|error| {
        let dir = config.workdir.display();
        CliError::new(exit::INVALID, format!("cannot create {dir}: {error}"))
    })?;
    let client = Client::new(&backend(connection, &config))?;
    let state = match State::load(&config.state_file())? {
        Some(state) => state,
        None => baseline(&config, args.run_now).await?,
    };
    let forced = config.triggers.iter().filter(|_| args.run_now);
    let mut watcher = Watcher {
        forced: forced.map(|t| t.name.clone()).collect(),
        local: start_local(&config, &client).await?,
        config,
        client,
        state,
        stop: stop_signal(),
        last_change: None,
    };
    let result = Box::pin(watcher.watch(args.once)).await;
    if let Some(local) = &mut watcher.local {
        local.stop().await;
    }
    result?;
    Ok(exit::OK)
}

/// With `backend.config`, makes sure that a backend answers, and starts
/// one when none does.
async fn start_local(
    config: &WatchConfig,
    client: &Client,
) -> Result<Option<LocalBackend>, CliError> {
    let Some(file) = &config.backend.config else {
        return Ok(None);
    };
    if !backend::is_local(client.base()) {
        return Err(CliError::new(
            exit::INVALID,
            format!(
                "backend.config starts only local backends, but the backend URL is {}; use \
                 http://127.0.0.1:<port>, or remove backend.config",
                client.base()
            ),
        ));
    }
    let mut local = LocalBackend::new(file.clone());
    local
        .ensure(client)
        .await
        .map_err(|error| CliError::new(exit::BACKEND, format!("{error:#}")))?;
    Ok(Some(local))
}

/// The backend settings of the file, else those of the command line.
fn backend(connection: &Connection, config: &WatchConfig) -> Connection {
    Connection {
        backend: config
            .backend
            .url
            .clone()
            .unwrap_or_else(|| connection.backend.clone()),
        token_env: config
            .backend
            .token
            .as_ref()
            .map_or_else(|| connection.token_env.clone(), |token| token.env.clone()),
    }
}

/// The first start records the current commits as tested.
async fn baseline(config: &WatchConfig, run_now: bool) -> anyhow::Result<State> {
    let mut state = State::default();
    for repository in &config.repositories {
        let commit = git::head(&repository.url, &repository.branch).await?;
        state.seen.insert(repository.name.clone(), commit);
    }
    if !run_now {
        for trigger in &config.triggers {
            let last = LastCycle {
                snapshot: state.seen.clone(),
                started_at: None,
                runs: Vec::new(),
            };
            state.triggers.insert(trigger.name.clone(), last);
        }
        log("first start: recorded the current commits; the next change starts a cycle");
    }
    state.save(&config.state_file())?;
    Ok(state)
}

/// A receiver that turns true on SIGINT or SIGTERM.
fn stop_signal() -> watch::Receiver<bool> {
    let (sender, receiver) = watch::channel(false);
    tokio::spawn(async move {
        let terminate = async {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut signal) => {
                    signal.recv().await;
                }
                Err(_) => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            () = terminate => {}
        }
        log("stopping; a running test run continues");
        let _ = sender.send(true);
    });
    receiver
}

/// What became of a submission.
enum Submitted {
    Run(RunId),
    Skipped,
    Stopped,
}

struct Watcher {
    config: WatchConfig,
    client: Client,
    state: State,
    /// Triggers that `--run-now` starts without a change.
    forced: BTreeSet<String>,
    stop: watch::Receiver<bool>,
    /// When the last change was seen.
    last_change: Option<Instant>,
    /// The backend that the watcher starts when none answers.
    local: Option<LocalBackend>,
}

impl Watcher {
    async fn watch(&mut self, once: bool) -> anyhow::Result<()> {
        let mut polled = false;
        while !*self.stop.borrow() {
            if self.state.cycle.is_some() {
                Box::pin(self.continue_cycle()).await?;
                continue;
            }
            if !(once && polled) {
                self.poll().await?;
                polled = true;
            }
            if let Some(trigger) = self.due(once)
                && self.start_cycle(&trigger).await?
            {
                continue;
            }
            if once || self.sleep(self.pause()).await {
                break;
            }
        }
        Ok(())
    }

    /// Reads the head of every branch and records the changes.
    async fn poll(&mut self) -> anyhow::Result<()> {
        let mut changed = false;
        for repository in &self.config.repositories {
            let commit = match git::head(&repository.url, &repository.branch).await {
                Ok(commit) => commit,
                Err(error) => {
                    log(&format!("cannot read {}: {error:#}", repository.name));
                    continue;
                }
            };
            let name = &repository.name;
            let old = self.state.seen.insert(name.clone(), commit.clone());
            if old.as_ref() != Some(&commit) {
                let old = old.as_deref().map_or("none", short);
                let new = short(&commit);
                log(&format!(
                    "change: {name} {} {old} -> {new}",
                    repository.branch
                ));
                changed = true;
            }
        }
        if changed {
            self.last_change = Some(Instant::now());
            self.state.save(&self.config.state_file())?;
        }
        Ok(())
    }

    /// The first trigger in configuration order that has a change since
    /// its last cycle, once no change arrived for the quiet period.
    fn due(&self, ignore_quiet: bool) -> Option<String> {
        if !ignore_quiet && !self.quiet_remaining().is_zero() {
            return None;
        }
        self.config
            .triggers
            .iter()
            .find(|trigger| self.forced.contains(&trigger.name) || self.changed(trigger))
            .map(|trigger| trigger.name.clone())
    }

    fn changed(&self, trigger: &Trigger) -> bool {
        let last = self.state.triggers.get(&trigger.name);
        trigger.on.iter().any(|repository| {
            last.is_none_or(|last| last.snapshot.get(repository) != self.state.seen.get(repository))
        })
    }

    fn quiet_remaining(&self) -> Duration {
        let quiet = Duration::from_secs(self.config.quiet_period_secs);
        self.last_change
            .map_or(Duration::ZERO, |at| quiet.saturating_sub(at.elapsed()))
    }

    /// Time to the next poll: the interval, or less when the quiet period
    /// ends earlier.
    fn pause(&self) -> Duration {
        let interval = Duration::from_secs(self.config.poll_interval_secs);
        match self.quiet_remaining() {
            remaining if remaining.is_zero() => interval,
            remaining => remaining.min(interval),
        }
    }

    /// Puts every clone at the snapshot and records the new cycle. Returns
    /// false when a clone cannot be updated; the next poll tries again.
    async fn start_cycle(&mut self, trigger: &str) -> anyhow::Result<bool> {
        let snapshot: Snapshot = self.state.seen.clone();
        if let Err(error) = self.update_clones(&snapshot).await {
            if !*self.stop.borrow() {
                log(&format!("trigger {trigger}: {error:#}"));
            }
            return Ok(false);
        }
        log(&format!(
            "trigger {trigger}: cycle started at {}",
            describe(&snapshot)
        ));
        let last = LastCycle {
            snapshot,
            started_at: Some(OffsetDateTime::now_utc()),
            runs: Vec::new(),
        };
        self.state.triggers.insert(trigger.to_owned(), last);
        self.state.cycle = Some(Cycle {
            trigger: trigger.to_owned(),
            next: 0,
            run: None,
        });
        self.forced.remove(trigger);
        self.state.save(&self.config.state_file())?;
        Ok(true)
    }

    async fn update_clones(&self, snapshot: &Snapshot) -> anyhow::Result<()> {
        for repository in &self.config.repositories {
            if *self.stop.borrow() {
                anyhow::bail!("stopping");
            }
            let name = &repository.name;
            let dir = self.config.repos_dir().join(name);
            let commit = snapshot.get(name).with_context(|| {
                format!("cannot update {name}: its branch head is not known yet")
            })?;
            Box::pin(git::checkout(
                &dir,
                &repository.url,
                &repository.branch,
                commit,
            ))
            .await
            .with_context(|| format!("cannot update {name}"))?;
        }
        Ok(())
    }

    /// Submits or waits for the next plan of the cycle in progress.
    async fn continue_cycle(&mut self) -> anyhow::Result<()> {
        let Some(cycle) = self.state.cycle.clone() else {
            return Ok(());
        };
        let Some(trigger) = self.config.trigger(&cycle.trigger).cloned() else {
            log(&format!(
                "trigger {} is no longer configured; its cycle ends",
                cycle.trigger
            ));
            return self.end_cycle();
        };
        let Some(entry) = trigger.plans.get(cycle.next) else {
            log(&format!("trigger {}: cycle finished", trigger.name));
            return self.end_cycle();
        };
        let run = match cycle.run {
            Some(run) => run,
            None => match self.submit(&trigger, cycle.next, entry).await? {
                Submitted::Run(run) => run,
                Submitted::Skipped => return self.advance(),
                Submitted::Stopped => return Ok(()),
            },
        };
        self.finish_plan(&trigger, entry, &run).await
    }

    /// Waits for the run of a plan and moves to the next plan.
    async fn finish_plan(
        &mut self,
        trigger: &Trigger,
        entry: &PlanEntry,
        run: &RunId,
    ) -> anyhow::Result<()> {
        let Some(outcome) = self.wait(run).await else {
            return Ok(());
        };
        let name = &trigger.name;
        log(&format!(
            "trigger {name}: run {run} of {} ended {outcome}",
            entry.plan
        ));
        self.record_outcome(run, outcome);
        self.advance()
    }

    fn end_cycle(&mut self) -> anyhow::Result<()> {
        self.state.cycle = None;
        self.state.save(&self.config.state_file())
    }

    fn advance(&mut self) -> anyhow::Result<()> {
        if let Some(cycle) = &mut self.state.cycle {
            cycle.next += 1;
            cycle.run = None;
        }
        self.state.save(&self.config.state_file())
    }

    /// Adds the record of a plan to the cycle in progress.
    fn record(&mut self, record: PlanRun) {
        let trigger = self.state.cycle.as_ref().map(|c| c.trigger.clone());
        if let Some(last) = trigger.and_then(|t| self.state.triggers.get_mut(&t)) {
            last.runs.push(record);
        }
    }

    /// Records the outcome of `run` in the cycle in progress.
    fn record_outcome(&mut self, run: &RunId, outcome: Outcome) {
        let runs = self
            .state
            .triggers
            .values_mut()
            .flat_map(|last| &mut last.runs);
        if let Some(record) = runs.into_iter().find(|r| r.run.as_ref() == Some(run)) {
            record.outcome = Some(outcome);
        }
    }

    /// Submits plan `index` of `trigger` with a key that makes a repeated
    /// submission return the run already accepted.
    async fn submit(
        &mut self,
        trigger: &Trigger,
        index: usize,
        entry: &PlanEntry,
    ) -> anyhow::Result<Submitted> {
        let bundle = match self.load_plan(&trigger.name, &entry.plan) {
            Ok(bundle) => bundle,
            Err(reason) => return self.skip(trigger, entry, &reason),
        };
        let key = self.key(&trigger.name, index);
        let request = SubmitRun {
            bundle,
            environment: entry.environment.clone(),
            policy: entry.policy.clone(),
        };
        loop {
            match self.send(&request, &key).await {
                Ok(run) => return self.accepted(trigger, entry, run),
                Err(error) if error.retryable || error.code == exit::UNCERTAIN => {
                    log(&format!(
                        "trigger {}: cannot submit {} yet: {}",
                        trigger.name, entry.plan, error.message
                    ));
                    self.ensure_backend().await;
                    if self.sleep(Duration::from_secs(RETRY_SECS)).await {
                        return Ok(Submitted::Stopped);
                    }
                }
                Err(error) => return self.skip(trigger, entry, &error.message),
            }
        }
    }

    /// Reads a plan from the clones; the error is why it cannot run.
    fn load_plan(&self, trigger: &str, plan: &str) -> Result<PlanBundle, String> {
        let path = self.config.repos_dir().join(plan);
        if !path.is_file() {
            return Err(self.missing(trigger, plan));
        }
        bundle::load(&path).map_err(|error| format!("not a valid plan: {error}"))
    }

    /// Why a plan path finds no file, with the commit of its clone.
    fn missing(&self, trigger: &str, plan: &str) -> String {
        let (repository, path) = plan.split_once('/').unwrap_or((plan, ""));
        let snapshot = self.state.triggers.get(trigger).map(|l| &l.snapshot);
        let commit = snapshot
            .and_then(|s| s.get(repository))
            .map_or("?", |c| short(c));
        format!("{path} does not exist in {repository} at {commit}; check the plan path")
    }

    /// `watch:<trigger>:<snapshot hash>:<plan index>`.
    fn key(&self, trigger: &str, index: usize) -> String {
        let snapshot = self.state.triggers.get(trigger).map(|l| &l.snapshot);
        let hash = snapshot_hash(snapshot.unwrap_or(&Snapshot::new()));
        format!("watch:{trigger}:{hash}:{index}")
    }

    async fn send(&self, request: &SubmitRun, key: &str) -> Result<RunId, CliError> {
        let headers = [(IDEMPOTENCY_KEY, key.to_owned())];
        let (_, run) = self
            .client
            .request::<_, RunSummary>(Method::POST, "/runs", Some(request), &headers)
            .await?;
        Ok(run.id)
    }

    fn accepted(
        &mut self,
        trigger: &Trigger,
        entry: &PlanEntry,
        run: RunId,
    ) -> anyhow::Result<Submitted> {
        log(&format!(
            "trigger {}: run {run} submitted for {}",
            trigger.name, entry.plan
        ));
        self.record(PlanRun {
            plan: entry.plan.clone(),
            run: Some(run.clone()),
            outcome: None,
            skipped: None,
        });
        if let Some(cycle) = &mut self.state.cycle {
            cycle.run = Some(run.clone());
        }
        self.state.save(&self.config.state_file())?;
        Ok(Submitted::Run(run))
    }

    fn skip(
        &mut self,
        trigger: &Trigger,
        entry: &PlanEntry,
        reason: &str,
    ) -> anyhow::Result<Submitted> {
        log(&format!(
            "trigger {}: {} skipped: {reason}",
            trigger.name, entry.plan
        ));
        self.record(PlanRun {
            plan: entry.plan.clone(),
            run: None,
            outcome: None,
            skipped: Some(reason.to_owned()),
        });
        Ok(Submitted::Skipped)
    }

    /// Waits until `run` ends; `None` when the watcher stops first. A run
    /// that the backend no longer knows counts as `error`.
    async fn wait(&mut self, run: &RunId) -> Option<Outcome> {
        let interval = Duration::from_secs(self.config.poll_interval_secs.min(10));
        loop {
            match self.client.get::<RunSummary>(&format!("/runs/{run}")).await {
                Ok(summary) if summary.lifecycle == Lifecycle::Finished => {
                    return Some(summary.outcome.unwrap_or(Outcome::Error));
                }
                Ok(_) => {}
                Err(error) if error.retryable => {
                    log(&format!("cannot read run {run} yet: {}", error.message));
                    self.ensure_backend().await;
                }
                Err(error) => {
                    log(&format!("cannot read run {run}: {}", error.message));
                    return Some(Outcome::Error);
                }
            }
            if self.sleep(interval).await {
                return None;
            }
        }
    }

    /// Starts the local backend again when it is configured and stopped.
    async fn ensure_backend(&mut self) {
        if let Some(local) = &mut self.local
            && let Err(error) = local.ensure(&self.client).await
        {
            log(&format!("{error:#}"));
        }
    }

    /// Sleeps; true when the watcher must stop.
    async fn sleep(&mut self, duration: Duration) -> bool {
        tokio::select! {
            () = tokio::time::sleep(duration) => *self.stop.borrow(),
            _ = self.stop.changed() => true,
        }
    }
}

/// `saci watch status`: prints the state file.
///
/// # Errors
///
/// A [`CliError`] for an invalid configuration or state file.
pub fn status(config: &Path) -> Result<u8, CliError> {
    let config = WatchConfig::load(config)?;
    let Some(state) = State::load(&config.state_file())? else {
        println!(
            "The watcher has not started: {} does not exist.",
            config.state_file().display()
        );
        return Ok(exit::OK);
    };
    println!("Commits seen:");
    for (name, commit) in &state.seen {
        println!("  {name:<24} {}", short(commit));
    }
    for trigger in &config.triggers {
        print_trigger(&trigger.name, state.triggers.get(&trigger.name));
    }
    if let Some(cycle) = &state.cycle {
        let run = cycle
            .run
            .as_ref()
            .map_or_else(|| "nothing yet".to_owned(), ToString::to_string);
        println!(
            "Waiting for: {run} (trigger {}, plan {})",
            cycle.trigger,
            cycle.next + 1
        );
    }
    Ok(exit::OK)
}

fn print_trigger(name: &str, last: Option<&LastCycle>) {
    let Some(last) = last else {
        println!("Trigger {name}: no cycle yet");
        return;
    };
    let Some(started) = last.started_at else {
        println!(
            "Trigger {name}: no cycle yet; baseline {}",
            describe(&last.snapshot)
        );
        return;
    };
    let started = started.format(&Rfc3339).unwrap_or_default();
    println!(
        "Trigger {name}: last cycle {started} at {}",
        describe(&last.snapshot)
    );
    for record in &last.runs {
        let state = match (&record.run, record.outcome, &record.skipped) {
            (_, _, Some(reason)) => format!("skipped: {reason}"),
            (Some(run), Some(outcome), _) => format!("{run} {outcome}"),
            (Some(run), None, _) => format!("{run} running"),
            (None, _, None) => "not submitted".to_owned(),
        };
        println!("  {:<48} {state}", record.plan);
    }
}

/// `name@commit` for every repository of a snapshot.
fn describe(snapshot: &Snapshot) -> String {
    snapshot
        .iter()
        .map(|(name, commit)| format!("{name}@{}", short(commit)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn short(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}

/// One log line on standard error, with the time.
fn log(message: &str) {
    let now = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default();
    eprintln!("{now} {message}");
}
