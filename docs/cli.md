# The `saci` command line

`saci` is the client. It authors plans offline, starts a backend, and
submits plans to a local or remote backend through the [REST API](api.md).
It never starts workers or containers itself: the backend does.

## Installing

To build the binaries in release mode and install them, run this command
from the repository root:

```sh
make install                      # installs in /usr/local/bin
make install BINDIR=$HOME/bin     # installs in a directory that you select
make install PREFIX=/opt/saci     # installs in /opt/saci/bin
```

The target installs `saci`, `saci-backend`, `saci-worker`,
`saci-script-runner` ([MCP script runner](script-runner.md)) and
`saci-julia` ([Julia 1 judge](julia-judge.md)). It uses only
`cargo` and the standard `install` tool, and works on Linux and macOS.

Do not run `sudo make install`. The target builds as the current user. If
you do not have write access to the directory, the target uses `sudo` only
to copy the binaries, and `sudo` asks for your password. To use a different
command, set `SUDO`, for example `make install SUDO=doas`. To disable it,
set `SUDO=`. Packagers can set `DESTDIR` to install into a staging root.

To embed the [web console](web-ui.md), run `make web` before
`make install`. To remove the installed binaries, run `make uninstall` with
the same `PREFIX` or `BINDIR`.

## Connecting to a backend

Every command that talks to a backend accepts:

| Option | Default | Meaning |
| --- | --- | --- |
| `--backend <url>` | `SACI_BACKEND_URL`, else `http://127.0.0.1:8080` | Backend base URL. |
| `--token-env <name>` | `SACI_TOKEN` | Environment variable holding the client token. |

When the backend sets `backend.auth.token_env` (required when it listens
beyond loopback, see [configuration](configuration.md)), export the same
token in `SACI_TOKEN` on the client. The token is sent as a bearer token and
never printed.

## `saci backend start`

```sh
export OPENROUTER_API_KEY=...        # whatever llm.api_key: env:NAME names
saci backend start --config saci.yaml
```

Reads and validates the configuration, opens the database in
`backend.data_dir`, and serves the API on `backend.listen` in the foreground
until Ctrl-C or SIGTERM. Logs go to standard error; `RUST_LOG` adjusts them.
The `saci-backend` binary does the same and exists for container images.

An invalid configuration, a missing client token variable or an unusable
listen address or data directory exits 2 with the offending setting.

## `saci check`

```sh
saci check plan.md [--strict] [--original <dir>]
```

`saci check` reads `plan.md` and each procedure that it invokes. It
validates them with the same rules as `saci run` ([test plans](test-plans.md))
and prints a summary. It works offline. It does not connect to a backend.

```text
$ saci check plans/orders.md
plans/orders.md is valid: "Orders", 6 units (4 tests, 2 steps), 1 procedure, 0 warnings.
```

If the plan is not valid, the command exits 2. It shows each problem with
its file and line.

The command also writes a warning to standard error for valid content that
does not run:

- A phase that has no tests or steps.
- A `Follow [...](...)` paragraph outside a test or step, for example in
  Prerequisites. It does not invoke the procedure.
- A fenced block in a phase but outside a test or step.
- A fenced block in Prerequisites. The model sees it as context. The worker
  does not run it as a step.
- A fenced block or block quote after `**Expected result:**`. The parser
  ignores it.

Warnings do not change the exit code. With `--strict`, the command exits 2
if there are warnings. Use `--strict` in CI and when you convert plans from
a different format.

Use `--original <dir>` when you convert a plan from a different format.
`<dir>` is a copy of the original documents, with the same layout as the
plan directory. The command then compares the code blocks of all documents
in the bundle with the code blocks of the originals at the same paths. It
finds code blocks at all levels, also in lists and block quotes. A code
block can move to a different document. If a code block is missing,
changed or new, the command exits 2 and shows each one. The
[`saci-convert-test-plan` skill](../skills/saci-convert-test-plan/SKILL.md)
uses this check to make sure that a conversion keeps all commands.

## `saci skill`

```sh
saci skill list
saci skill install <name>... --agent <agent> [--agent <agent>] [--global] [--force]
saci skill install --all --dir <dir>
```

Saci contains skills for coding agents. A skill teaches an agent to do a
Saci task. The skills use the `SKILL.md` format that most coding agents
read. These commands work offline.

| Skill | Teaches the agent to |
| --- | --- |
| `saci-convert-test-plan` | Convert an existing Markdown test plan to the Saci format, with all commands unchanged. |
| `saci-start-backend` | Start a local backend in the background if no backend answers at the backend URL, and wait until it answers. The skill contains the bash script `ensure-backend.sh`. |
| `saci-run-test-plan` | Validate a plan, start the backend with `saci-start-backend` if necessary, submit the run, follow it to the end and summarize the report. |
| `saci-investigate` | Find the failed run (a run ID, or the most recent run that did not pass), start a [failure investigation](investigations.md), follow it to the end and summarize the probable cause, the cited code and the bug. |

Install `saci-run-test-plan` and `saci-investigate` with `saci-start-backend`.

`saci skill list` shows the name and description of each skill.

`saci skill install` writes each skill to `<skills directory>/<name>/`.
Use `--all` to install all skills. Use `--agent` to select the coding
agent. You can use `--agent` more than one time. Without `--global`, the
command installs into the project in the current directory. With
`--global`, it installs into your home directory, and the skill is
available in all projects.

| `--agent` | Project directory | `--global` directory |
| --- | --- | --- |
| `claude` (Claude Code) | `.claude/skills` | `~/.claude/skills` |
| `codex` (OpenAI Codex) | `.agents/skills` | `~/.agents/skills` |
| `cursor` | `.cursor/skills` | `~/.cursor/skills` |
| `copilot` (GitHub Copilot) | `.github/skills` | `~/.copilot/skills` |
| `gemini` (Gemini CLI) | `.gemini/skills` | `~/.gemini/skills` |
| `opencode` | `.opencode/skills` | `~/.config/opencode/skills` |
| `agents` (shared location) | `.agents/skills` | `~/.agents/skills` |

Codex, Cursor, Copilot, Gemini CLI and OpenCode also read the shared
`.agents/skills` location. For a different agent, use `--dir` to give its
skills directory.

If a skill is already installed and is the same, the command reports that
it is up to date. If the installed skill is different, for example because
you changed it or because it is from an older Saci version, the command
does not change it and exits 2. Use `--force` to replace it. An unknown
skill name also exits 2.

## `saci run`

```sh
saci run plan.md --environment staging [--policy keep-going] \
    [--idempotency-key <key>] [--json]
```

1. Reads `plan.md` and every procedure it invokes into a bundle and validates
   it locally ([test plans](test-plans.md)). Invalid plans exit 2 before the
   backend is contacted.
2. Submits the bundle with an `Idempotency-Key`, a fresh UUID unless
   `--idempotency-key` is given.
3. Prints the accepted run ID on standard output, and a one-line summary on
   standard error; `--json` prints the run summary instead.

`saci run` returns once the backend accepts the run. With `--follow` it then
behaves like `saci follow` and exits with the run's outcome.

### When the reply is lost

If the connection breaks after the request was sent, the CLI cannot know
whether the run was accepted. It exits 4 and prints the key it used:

```text
error: the reply from http://127.0.0.1:8080 was lost (...); the request may or may not have taken effect.
Repeat the same command with --idempotency-key 0f8c...: the backend returns the run it already accepted instead of starting a second one.
```

Repeating the command with that key is always safe: the backend answers with
the run it already accepted (the summary says "already accepted for this
idempotency key"), or accepts it now. Reusing a key with a different plan,
environment or policy is refused with `idempotency_conflict`.

## Following a run

```sh
saci list [--limit 20]                 # recent runs, newest first
saci status <run-id>                   # state, progress, cancellation, outcome
saci follow <run-id> [--after <seq>]   # progress until the run finishes
saci report <run-id>                   # units, attempts, criteria and evidence
saci cancel <run-id> [--wait]          # request cancellation
```

Every command accepts `--json`, which prints the API's own body instead of
text: the run summary for `status`, `cancel` and `run`, the report for
`report`, and for `follow` one event per line (JSON Lines). Text goes to
standard output; notes about waiting and reconnecting go to standard error.

### Waiting

`saci follow` prints each event as one line, for example:

```text
[2] 1.1 attempt 1 started
[3] 1.1 attempt 1   ev-1 shell `curl $URL/health` exited 0 (12 ms)
[4] 1.1 attempt 1 passed (model: 2 requests, 3120 ms, tokens: 5200 in (4096 cached), 310 out; tools: 12 ms): the service answered 200
[5] 1.1 passed
[10] run finished failed: stopped after 1.2 ended failed
```

The number in brackets is the event's sequence number. A finished attempt
shows where it spent its time: the number of model requests and the time
spent on them, the tokens that the provider counted, and the time spent
in tool calls. The tokens show only when the provider reports them, and
`cached` only when it reports prompt cache hits. The rest of the attempt
is worker overhead. `saci report` shows the same values for each attempt.

Each request waits up
to 30 seconds on the backend for new events (a long poll), so a quiet run
costs one request per 30 seconds. When the connection fails or the backend
answers with a server error, `follow` says so on standard error and
reconnects every two seconds, continuing after the last event it printed:
no event is skipped or printed twice. After five minutes without a
successful request it gives up (exit 3) and prints the command that resumes
where it stopped, `saci follow <run-id> --after <seq>`.

Interrupting `follow` (Ctrl-C) stops following, not the run; `saci cancel`
stops the run.

### Cancellation

`saci cancel <run-id>` records the request and returns at once. The worker
stops after its current action (a running tool gets a few seconds to finish
and record its evidence), skips the remaining units and reports the end.
Until then `saci status` shows the request as *pending*; afterwards as
*completed*, with the run's outcome. `--wait` follows the run quietly until
it has stopped and exits with its outcome, normally 7 (`cancelled`). A run
that finished before the request reached it keeps its outcome, and
cancelling a finished run exits 2 ("the run already finished"). Repeating a
cancel request is harmless.

## Outcome actions

`saci report` lists a finished run's [outcome actions](actions.md) after
the report. Their delivery never changes the run's outcome or exit code.
The `saci actions` commands work with them directly:

| Command | Does |
| --- | --- |
| `saci actions list` | Every action waiting for approval, across runs. |
| `saci actions list <run-id>` | A run's actions and their states. |
| `saci actions show <run-id> <name>` | One action: its state, decision, result or error, idempotency key, the exact arguments it sends, their digest, and its history. |
| `saci actions approve <run-id> <name> [--digest D] [--note N]` | Approves the recorded arguments; the backend then sends them. |
| `saci actions reject <run-id> <name> [--note N]` | Rejects the action; it is never sent. |
| `saci actions redispatch <run-id> <name> [--digest D] [--note N]` | Sends an `unknown` or `error` action again with the same arguments. |
| `saci actions reconcile <run-id> <name> --as completed\|error [--note N]` | Records what the remote system shows for an `unknown` action; nothing is sent. |

States are shown as *pending approval*, *queued*, *approved, queued*,
*executing*, *rejected*, *completed*, *error* and *unknown, needs
reconciliation* ([retries and restarts](actions.md#retries-and-restarts)).
`approve`, `reject` and `redispatch` print the arguments they act on. To approve only the payload you
reviewed earlier with `show`, pass its `--digest`: the backend refuses the
decision for any other payload (exit 2). Deciding on an action that is not
waiting for approval, or redispatching or reconciling one in another
state, also exits 2. All commands accept `--json`.

## Investigations

`saci investigate <run-id>` starts a [failure investigation](investigations.md)
of a finished run and prints the investigation ID. The backend must have
an `investigations` section.

| Option | Does |
| --- | --- |
| `--unit <key>` | The failed unit. Without it, the first unit that failed, ended in error or was blocked. |
| `--source <name>[=<ref>]` | A configured repository to clone, with an optional branch, tag or commit. Repeat it for more. Without it, every configured repository at its configured ref. |
| `--idempotency-key <key>` | Makes a repeated request safe; generated when omitted. |
| `--json` | Prints the investigation as JSON. |

A run and unit have one current investigation: a second request prints
its ID. After an investigation that ended in `error` or `cancelled`, a
request starts a new one. The investigation ID is a run ID, so `saci follow`,
`saci report` and `saci cancel` work with it.

`saci investigations show <id>` prints the state, the repositories and,
once the agent concluded, the summary, the cloned commits, the
hypotheses with their code locations and evidence, and the bug. It exits
0 for `concluded`, 5 for `inconclusive`, 6 for `error`, 7 for `cancelled`
and 8 while the investigation runs.

## Watching repositories

`saci watch --config watch.yaml` polls Git branches and runs test plans
when they change, one cycle at a time. `--once` polls one time and exits,
`--run-now` starts a cycle of every trigger without a change, and
`saci watch status` prints the state. See
[running plans when code changes](watch.md).

## Exit codes

| Code | Meaning | What to do |
| --- | --- | --- |
| 0 | Success; for `status`, `follow`, `report`, `cancel --wait` and `run --follow`, the run passed. | |
| 1 | The run finished `failed`: an expected result did not hold. | Read `saci report`. |
| 2 | Invalid input: plan, configuration, arguments, an unknown environment, policy, run or action, a run that already finished, an action decision, redispatch or reconciliation the backend refused, or an investigation request the backend refused. | Fix the input named in the message. |
| 3 | The backend is unreachable, refused the credentials, or failed. | Start the backend, check `--backend`, or set `SACI_TOKEN`; the message says which. |
| 4 | The outcome of a request that changes state is unknown. | Repeat it as the message says. |
| 5 | The run finished `blocked` or `inconclusive`, or units were skipped. | Read `saci report` for what was not established. |
| 6 | The run finished `error`: infrastructure, the model provider or limits. | Read the reason; the run says nothing about the target. |
| 7 | The run finished `cancelled`. | |
| 8 | The run has not finished (`status`, `report` and `investigations show` only). | Wait with `saci follow`. |

Codes 0, 1 and 5 to 8 come from the run's summary outcome, which the backend
computes; the CLI never combines unit results itself.
