# The `saci` command line

`saci` is the client. It authors plans offline, starts a backend, and
submits plans to a local or remote backend through the [REST API](api.md).
It never starts workers or containers itself: the backend does.

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
export OPENROUTER_API_KEY=...        # whatever llm.api_key_env names
saci backend start --config saci.yaml
```

Reads and validates the configuration, opens the database in
`backend.data_dir`, and serves the API on `backend.listen` in the foreground
until Ctrl-C or SIGTERM. Logs go to standard error; `RUST_LOG` adjusts them.
The `saci-backend` binary does the same and exists for container images.

An invalid configuration, a missing client token variable or an unusable
listen address or data directory exits 2 with the offending setting.

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
[4] 1.1 attempt 1 passed: the service answered 200
[5] 1.1 passed
[10] run finished failed: stopped after 1.2 ended failed
```

The number in brackets is the event's sequence number. Each request waits up
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
| `saci actions show <run-id> <name>` | One action: its state, decision, result or error, the exact arguments it sends, and their digest. |
| `saci actions approve <run-id> <name> [--digest D] [--note N]` | Approves the recorded arguments; the backend then sends them. |
| `saci actions reject <run-id> <name> [--note N]` | Rejects the action; it is never sent. |

States are shown as *pending approval*, *queued*, *approved, queued*,
*executing*, *rejected*, *completed* and *error*. `approve` and `reject`
print the arguments they decide on. To approve only the payload you
reviewed earlier with `show`, pass its `--digest`: the backend refuses the
decision for any other payload (exit 2). Deciding on an action that is not
waiting for approval also exits 2. All commands accept `--json`.

## Exit codes

| Code | Meaning | What to do |
| --- | --- | --- |
| 0 | Success; for `status`, `follow`, `report`, `cancel --wait` and `run --follow`, the run passed. | |
| 1 | The run finished `failed`: an expected result did not hold. | Read `saci report`. |
| 2 | Invalid input: plan, configuration, arguments, an unknown environment, policy, run or action, a run that already finished, or an action decision the backend refused. | Fix the input named in the message. |
| 3 | The backend is unreachable, refused the credentials, or failed. | Start the backend, check `--backend`, or set `SACI_TOKEN`; the message says which. |
| 4 | The outcome of a request that changes state is unknown. | Repeat it as the message says. |
| 5 | The run finished `blocked` or `inconclusive`, or units were skipped. | Read `saci report` for what was not established. |
| 6 | The run finished `error`: infrastructure, the model provider or limits. | Read the reason; the run says nothing about the target. |
| 7 | The run finished `cancelled`. | |
| 8 | The run has not finished (`status` and `report` only). | Wait with `saci follow`. |

Codes 0, 1 and 5 to 8 come from the run's summary outcome, which the backend
computes; the CLI never combines unit results itself.
