# Running plans when code changes

`saci watch` runs test plans when new commits arrive on watched Git
branches, for example after a pull request merge. It runs on the Saci
host and polls the repositories. GitHub does not need access to the host,
and no webhook or GitHub Actions workflow is necessary.

The [design](plans/source-watch.md) gives the reasons for each decision.

## How it works

1. **Poll.** At each `poll_interval_secs`, the watcher runs
   `git ls-remote` for the branch of each repository.
2. **Wait for quiet.** After a change, the watcher waits until no
   repository changed for `quiet_period_secs`. Related merges in several
   repositories thus give one cycle.
3. **Snapshot.** The watcher puts each of its clones at the newest commit
   of its branch. The set of commits is the snapshot.
4. **Run.** For the first trigger with a change, the watcher reads each
   plan from the clones, submits it to the backend and waits until the run
   ends. Then it submits the next plan. A failed plan does not stop the
   next plans.

The watcher runs one cycle at a time. It does not stop a run when new
changes arrive. After the cycle, it starts one new cycle with the newest
snapshot. Snapshots between the two are not tested.

The watcher does not build or deploy the system under test, and gives no
versions to the run. A plan that needs a version states it, for example an
image tag or a `git clone` of a branch in the worker.

## Configuration

```yaml
backend:
  url: http://127.0.0.1:18080       # optional; else --backend or SACI_BACKEND_URL
  token: { env: SACI_TOKEN }        # optional; else --token-env
  config: saci.yaml                 # optional; start this backend when none answers
workdir: ~/.local/state/saci-watch
poll_interval_secs: 60
quiet_period_secs: 180
repositories:
  - name: wanaku
    url: https://github.com/wanaku-ai/wanaku.git
    branch: main
  - name: wanaku-tests
    url: git@github.com:wanaku-ai/wanaku-tests.git
    branch: main
triggers:
  - name: operator
    on: [wanaku, wanaku-tests]
    plans:
      - plan: wanaku-tests/tests/plans/operator-basic-functionality.md
        environment: openshift
        policy: keep-going
```

| Key | Default | Meaning |
| --- | --- | --- |
| `backend.url` | `--backend` | Backend base URL. |
| `backend.token` | `--token-env` | `{ env: NAME }`: the variable that holds the client token. |
| `backend.config` | none | A backend configuration file. When no backend answers at the backend URL, the watcher starts `saci backend start --config <file>`. A relative path is relative to the watch configuration file. See [starting the backend](#starting-the-backend). |
| `workdir` | required | Directory for the clones (`repos/<name>`) and the state file (`state.json`). An absolute path, or a path that starts with `~/`. |
| `poll_interval_secs` | `60` | Seconds between two polls. Also the upper limit of the time between two checks of a run. |
| `quiet_period_secs` | `180` | Seconds without a change before a cycle starts. |
| `repositories[].name` | required | Clone directory name and the first part of plan paths. Letters, digits, `-`, `_` and `.`. |
| `repositories[].url` | required | A URL that `git clone` accepts. |
| `repositories[].branch` | required | The watched branch. |
| `triggers[].name` | required | Trigger name. It is part of the idempotency keys. |
| `triggers[].on` | required | Repositories whose changes start the trigger. |
| `triggers[].plans[].plan` | required | `<repository>/<path>`: the entry document of a plan in the clones. |
| `triggers[].plans[].environment` | required | A configured environment of the backend. |
| `triggers[].plans[].policy` | `default` | A configured run policy of the backend. |

A repository that only supplies plans, for example a test repository, can
be left out of every `on`. Its clone is still updated for each cycle. When
two triggers have a change, they run one after the other, in
configuration order.

**Credentials.** `git` uses the credentials that it has on the host, for
example an SSH agent or a credential helper. `git` never asks for a
password: a repository that needs one that `git` does not have gives a
log line at each poll.

**The work directory.** The watcher owns its clones. For each cycle it
runs `git checkout --force` and `git clean -ffdx` in them, so local
changes are lost. Do not edit the clones. The watcher refuses a `workdir`
that is the home directory or is in a Git working tree.

## Starting the backend

With `backend.config`, the watcher and the backend run as one service:

- At start, the watcher sends `GET /api/v1/health` to the backend URL. When
  a backend answers, the watcher uses it and starts nothing.
- When no backend answers, the watcher starts
  `saci backend start --config <file>` and waits up to 60 seconds until it
  answers. Else the command exits 3. The backend writes its log to the
  standard error of the watcher.
- When the backend stops during a cycle, the watcher starts it again
  before it submits or checks the next run.
- When the watcher stops, it sends SIGTERM to the backend that it started,
  and waits up to 30 seconds. A backend that the watcher did not start
  continues.

The watcher starts only local backends. The backend URL must start with
`http://127.0.0.1:`, `http://localhost:` or `http://[::1]:`, and must agree
with `backend.listen` in the backend configuration.

## Command line

```sh
saci watch --config watch.yaml [--once] [--run-now]
saci watch status --config watch.yaml
```

| Option | Does |
| --- | --- |
| `--config <file>` | The configuration. Default: `watch.yaml`. |
| `--once` | Polls one time, runs the triggers that have a change, then exits. The quiet period does not apply. Use it with cron or a systemd timer, or to test a configuration. |
| `--run-now` | Starts a cycle of every trigger, also without a change. |

`saci watch` writes one log line on standard error for each event: a
change, a cycle start, a submitted run, an ended run and a skipped plan.
On SIGINT or SIGTERM it stops after its current step. A running test run
continues.

`saci watch status` reads the state file and prints the last commits
seen, the last cycle of each trigger with its runs and outcomes, and the
run that the watcher waits for.

## First start, restarts and duplicates

- **First start.** Without a state file, the watcher records the current
  commits and starts no cycle. The next change starts one. Use
  `--run-now` to start a cycle at once.
- **A trigger added later** has no recorded cycle. It starts at the next
  poll.
- **Restarts.** The state file records the cycle in progress. After a
  restart, the watcher waits for the recorded run, then continues with
  the next plan.
- **No duplicate runs.** Each submission has the idempotency key
  `watch:<trigger>:<snapshot hash>:<plan index>`. When a reply was lost,
  the watcher submits again with the same key, and the backend returns
  the run that it already accepted.

## Failures

| Failure | Result |
| --- | --- |
| `git ls-remote` fails | Log line. The repository is read again at the next poll. On the first start, the command exits 2. |
| A clone cannot be updated | Log line. No cycle starts. The next poll tries again. |
| A plan is missing or not valid | Log line. The plan is recorded as skipped, and the next plan runs. |
| The backend refuses a plan, for example an unknown environment | As for a plan that is not valid. |
| The backend is unreachable or a reply is lost | The watcher submits again every 10 seconds, with the same key. |
| The backend does not know a run that the watcher waits for | The run counts as `error`. |

The watcher sends no notifications. Use the backend's
[outcome actions](actions.md) to report run results.

## Starting the watcher with the host

Examples are in [`examples/watch`](../examples/watch):

- `watch.yaml`: a project in several repositories;
- `saci-watch.service`: a systemd user unit for Linux;
- `io.github.orpiske.saci-watch.plist`: a launchd agent for macOS.

On Linux:

```sh
mkdir -p ~/.config/saci ~/.config/systemd/user
cp examples/watch/watch.yaml ~/.config/saci/watch.yaml
cp saci.yaml ~/.config/saci/saci.yaml     # the backend configuration, for backend.config
cp examples/watch/saci-watch.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now saci-watch
journalctl --user -u saci-watch -f
```

On macOS, replace `USER` in the file with your user name, then:

```sh
cp examples/watch/io.github.orpiske.saci-watch.plist ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/io.github.orpiske.saci-watch.plist
```
