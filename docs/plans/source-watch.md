# Design: run test plans when code changes

Status: implemented ([#135](https://github.com/orpiske/midnight-saci/issues/135)). The user guide is [docs/watch.md](../watch.md).

## Goal

When code is merged into a watched branch on GitHub, Saci runs the
related test plans without a person. Two constraints apply:

- GitHub cannot reach the host where Saci runs. GitHub Actions and
  webhooks are not available. A process on that host must find the
  changes itself.
- The system under test can be split across several repositories. For
  example, Wanaku has a server, an operator and a test repository. A
  change in one repository, or related merges in several repositories,
  must give one test cycle against one consistent set of commits.

## Overview

A new command, `saci watch`, runs on the Saci host as a long-running
process. It does not change the backend: it is a client of the REST API,
like `saci run`.

```mermaid
flowchart LR
    gh["GitHub repositories"]
    subgraph host["Saci host"]
        watch["saci watch"]
        clones[("Local clones<br/>and state file")]
        backend["saci-backend"]
        worker["saci-worker"]
    end

    watch -- "git ls-remote (poll)" --> gh
    watch -- "git fetch, checkout" --> clones
    watch -- "POST /api/v1/runs, GET run" --> backend
    backend -- launch --> worker
```

One cycle:

1. **Poll.** At each interval, the watcher runs `git ls-remote` for the
   branch of each repository and compares the commit with the last
   commit that it saw.
2. **Wait for quiet.** When a repository changed, the watcher waits until
   no watched repository changed for `quiet_period_secs`. Related merges
   in several repositories thus give one cycle.
3. **Snapshot.** The watcher fetches every repository and checks out the
   current commit of its branch. The set of commits is the snapshot. All
   plans of the cycle are read from the same snapshot.
4. **Run.** The watcher reads each plan from the clones, submits it to the
   backend, and waits until the run ends before it submits the next plan.

## Versions under test

The plan states the versions that it tests, for example the image tags or
the commands that build or deploy the system under test. The watcher does
not build, deploy or give versions to the run. It only decides when to
run and gives the newest plans to the backend. A plan that tests the
newest code of a branch says so itself, for example with a `main` image
tag or a clone of the branch in the worker.

Thus the backend, the worker and the plan format do not change.

The watcher can also start a local backend when none answers
([#141](https://github.com/orpiske/midnight-saci/issues/141)). Then one
service runs both. See [starting the backend](../watch.md#starting-the-backend).

## Detecting changes

The watcher uses `git ls-remote <url> refs/heads/<branch>`, not the GitHub
REST API:

- It needs no API token and has no API rate limit. A private repository
  uses the credentials that `git` already uses on the host (SSH agent or
  credential helper).
- It works with every Git host, not only GitHub.
- A merge, a squash merge, a rebase merge and a direct push all change the
  head of the branch. The watcher does not need to know which one it was.

The cost is one small request for each repository at each interval. The
default interval is 60 seconds.

Rejected alternatives:

- **Webhooks through a relay** (for example smee.io). This adds an
  external service that receives the events, and events are lost while
  the host is down.
- **The GitHub REST API.** It gives the pull request number and author,
  but needs a token, has rate limits and works only with GitHub.

## The clones

The watcher owns its clones in `<workdir>/repos/<name>`. It creates a
clone when it is missing. To update a clone, it runs `git fetch` and
`git checkout --force --detach <commit>`, then `git clean -ffdx`. Local
changes in a clone are lost. Do not edit the clones by hand.

Clones that the watcher owns give a known state for each cycle. A
person's working copy can have local changes, another branch or build
output, and the watcher must not change it.

The clones are for the plans only. Workers mount no host directories
([launchers](../launchers.md)).

## Repositories and triggers

The configuration has one list of repositories and one list of triggers.
A trigger names the repositories that start it and the plans that it
runs. A repository that only supplies plans, for example the test
repository, is in the snapshot but does not need to start a trigger.

```yaml
backend:
  url: http://127.0.0.1:18080       # default: --backend or SACI_BACKEND_URL
  token: { env: SACI_TOKEN }        # optional
workdir: ~/.local/state/saci-watch  # clones and the state file
poll_interval_secs: 60
quiet_period_secs: 180
repositories:
  - name: wanaku
    url: https://github.com/wanaku-ai/wanaku.git
    branch: main
  - name: wanaku-operator
    url: https://github.com/wanaku-ai/wanaku-operator.git
    branch: main
  - name: wanaku-tests
    url: git@github.com:wanaku-ai/wanaku-tests.git
    branch: main
triggers:
  - name: operator
    on: [wanaku, wanaku-operator, wanaku-tests]
    plans:
      - plan: wanaku-tests/plans/operator-basic-functionality.md
        environment: openshift
        policy: keep-going
      - plan: wanaku-tests/plans/operator-upgrade.md
        environment: openshift
```

- `on` lists the repositories whose changes start the trigger. A change
  in a repository that no trigger names is ignored.
- `plan` is a path in the clones: the repository name, then the path in
  that repository.
- When two triggers start from the same change, they run one after the
  other, in configuration order.

## One cycle at a time

Test plans of one project often use one shared system under test, for
example one OpenShift namespace. Two cycles at the same time can break
each other. Thus:

- The watcher runs one cycle at a time, and the plans of a cycle one
  after the other.
- When changes arrive during a cycle, the watcher does not stop the
  current run. After the cycle, it starts one new cycle with the newest
  snapshot. Intermediate snapshots are not tested.
- The watcher waits for a run with `GET /api/v1/runs/{id}` until the run
  ends. A failed plan does not stop the next plans of the cycle.
- A plan that cannot be read or is not valid is logged and skipped. The
  next plans of the cycle still run.

## Restarts and duplicates

The watcher keeps its state in `<workdir>/state.json`: the last snapshot
that it saw, the last snapshot that it tested for each trigger with its
run IDs, and the run that it waits for. It writes the file to a temporary
file and renames it, so a crash cannot leave a partial file.

- **First start.** Without a state file, the watcher records the current
  commits and starts no cycle. `--run-now` starts a cycle at once.
- **Restart during a cycle.** The watcher reads the state, waits for the
  recorded run, then continues with the next plans.
- **No duplicate runs.** The `Idempotency-Key` of each submission is
  `watch:<trigger>:<snapshot hash>:<plan index>`. When the watcher
  submits a run again after a lost reply or a crash, the backend returns
  the run that it already accepted.

## Command line

```sh
saci watch --config watch.yaml [--once] [--run-now]
saci watch status --config watch.yaml
```

- `saci watch` polls until it is stopped. On SIGINT or SIGTERM it stops
  after the current request. It does not cancel a running test run.
- `--once` polls one time, runs a cycle when a change is found, and exits.
  This is for tests and for hosts that use cron or a systemd timer.
- `saci watch status` shows the snapshot, the last cycle of each trigger
  with its commits, runs and outcomes, and the run that the watcher waits
  for.
- Log lines go to standard error, one line for each event: change found
  (repository and commit), cycle started, run submitted, run ended.

The user guide gives a systemd user unit and a launchd agent to start the
watcher with the host.

## Where the code goes

`saci watch` is a module of `saci-cli`. The CLI already has the REST
client, the plan bundle loader and the backend settings. The watcher uses
the `git` program, as investigations do, and no Git library.

## Notifications

The watcher does not send notifications. The backend's
[outcome actions](../actions.md) already report run results, for example
by email.

## Security

- The configuration file is trusted. It gives the repositories that the
  watcher clones and the plans that it submits.
- Values from the repositories (branch heads, file contents) never become
  part of a command on the host. The watcher runs `git` with argument
  lists, never through a shell.
- The backend token comes from an environment variable, never from the
  file.
- `git clean -ffdx` removes files in the clones only. The watcher refuses
  a `workdir` that is a Git working tree or the home directory.

## Not in scope

- Build or deploy commands on the host. The plan does this when needed.
- Commits or other values given to the run. The plan states its versions.
- Pull requests that are not merged, tags and releases.
- Path filters (start a trigger only when some files change).
- More than one cycle at the same time.
- A console page for the watcher.

## Tickets

1. **Watcher core** ([#137](https://github.com/orpiske/midnight-saci/issues/137)): configuration, `git ls-remote` polling, the quiet
   period, the clones and the snapshot, the state file, submission with
   idempotency keys, waiting for runs, `--once`, `--run-now` and
   `saci watch status`. Tests use local bare repositories (`file://`
   URLs) and the `saci-testkit` backend fixtures.
2. **Docs and examples** ([#138](https://github.com/orpiske/midnight-saci/issues/138)): a user guide in `docs/watch.md`, the command in
   `docs/cli.md`, an example configuration for a project in several
   repositories in `examples/`, and the systemd and launchd units.

Ticket 2 depends on 1.
