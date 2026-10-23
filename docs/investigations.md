# Failure investigations

An investigation finds the probable cause of a failed unit in the source
code. An agent clones the configured repositories, reads the code that
serves the failed step, and ends with a summary, hypotheses that cite code
locations, and a bug link or the reason that it filed no bug. The design
is in [plans/investigations.md](plans/investigations.md).

An investigation is executed by a run. The investigation ID is the ID of
that run. Thus the run routes, `saci follow`, `saci report` and
`saci cancel` also work for an investigation. Investigations are not in
the run list: they have their own list.

## Configure investigations

Add an `investigations` section to the backend configuration (see
[configuration](configuration.md#investigations)). Without it, the backend
refuses every investigation request with `404 not_found`.

```yaml
environments:
  investigator:
    secrets:
      GIT_TOKEN:
        env: GIT_TOKEN_SOURCE      # read code and create issues only
investigations:
  environment: investigator
  repositories:
    - name: orders-service
      url: https://github.com/example/orders-service.git
      ref: main
      description: The orders REST service.
  prepare:
    - 'ripwire . --for="{{failure.title}}: {{failure.reason}}" --token-budget=4000'
  instructions: |
    File bugs in example/orders-service. Search for an open issue first.
    Find code with `ripwire . --for="..."` and `ripwire . --callers=NAME`.
    Read only the lines that it points to.
  limits:
    run_timeout_secs: 1800
    output_limit_bytes: 8192
```

The Saci image contains [ripwire](https://github.com/redhat-et/ripwire), a
code map tool that finds the code for a symptom in few tokens. The
`prepare` command above gives the agent a first map. The `instructions`
tell the agent to use it for more queries. Set `investigations.image` only
for an image with other tools, for example one built `FROM` the Saci image.

The investigation run uses:

- the parameters, secrets and MCP servers of `investigations.environment`
  only. It never gets the secrets of the investigated run's environment;
- the limits of that environment, with `investigations.limits` applied
  over them;
- `investigations.image` as the worker image when it is set, else
  `worker.image`;
- no judge and no outcome actions.

## Start an investigation

With the CLI:

```sh
saci investigate run-0f3c9a2b --unit 1.2 --source orders-service=v2.4.1
saci follow <investigation-id>
saci investigations show <investigation-id>
```

With the REST API, send `POST /api/v1/investigations`:

```json
{
  "run_id": "run-0f3c9a2b7d4e4f1a9b8c7d6e5f4a3b2c",
  "unit": "1.2",
  "sources": [{ "name": "orders-service", "ref": "v2.4.1" }]
}
```

- `unit` is optional. Without it, the backend selects the first unit with
  outcome `failed`, `error` or `blocked`.
- `sources` is optional. Without it, the agent gets every configured
  repository at its configured ref. Each name must be configured, and a
  `ref` must be a branch, tag or commit.

The backend refuses the request with `422 validation_failed` when the run
has not finished, the run is itself an investigation, the unit did not
fail, end in error or get blocked, or a source is not configured. It
refuses an unknown run with `404 not_found`.

## What the agent gets

The backend copies these items from the stored report of the investigated
run into the investigation specification:

- the plan title and the unit's action text;
- the unit's report: identity, expected results, outcome and attempts;
- the evidence that the attempts cite. For an attempt with an assessment,
  this is the evidence that the assessment cites. For an attempt without
  an assessment, this is its last three observations. Each captured text
  is cut to the investigation's `output_limit_bytes`. The evidence was
  redacted when it was stored.

The worker then clones the repositories and runs the `prepare` commands
before the first model request (see
[investigation mode](worker.md#investigation-mode)).

## One investigation for each unit

Each run and unit has one current investigation. A second request for the
same unit returns the current investigation with `200`, also with other
sources. When the current investigation finished with outcome `error` or
`cancelled`, for example because the worker could not start, a request
starts a new investigation with `202`. The run lists all of them.
A request with the `Idempotency-Key` of an earlier request returns that
request's investigation. The same key with another body is refused with
`409 idempotency_conflict`.

## Read the result

`GET /api/v1/investigations/{investigation_id}` and
`saci investigations show` give:

| Field | Meaning |
| --- | --- |
| `parent_run`, `unit` | The investigated run and unit. |
| `lifecycle` | As for a run: `accepted` to `finished`. |
| `outcome` | Once finished: `concluded`, `inconclusive`, `error` or `cancelled`. |
| `reason` | Why the run ended. |
| `repositories` | The repositories to clone, with the selected refs. |
| `result` | The agent's result: summary, cloned commits, hypotheses with code locations and evidence IDs, and the bug or the reason for none. |

An investigation that ends without a result, for example after a failed
clone, has outcome `error`, or `cancelled` when it was cancelled.
`saci investigations show` exits with code 0 for `concluded`, 5 for
`inconclusive`, 6 for `error`, 7 for `cancelled` and 8 while the
investigation runs.

`GET /api/v1/runs/{run_id}/investigations` lists the investigations of a
run, oldest first. `GET /api/v1/investigations` lists the investigations
of all runs, newest first.

## A2A

Other agents, CI jobs and outcome actions can start and follow
investigations with the [A2A protocol](https://a2a-protocol.org/)
(version 0.3.0, JSON-RPC 2.0 transport). The backend serves A2A only when
the `investigations` section is configured. Without it, both paths answer
`404`.

| Path | Use |
| --- | --- |
| `GET /.well-known/agent-card.json` | The agent card. It needs no token. It names the endpoint URL, the one skill `investigate-failure`, and the `bearer` security scheme when the backend has a client token. Streaming and push notifications are off. |
| `POST /a2a` | The JSON-RPC methods `message/send`, `tasks/get` and `tasks/cancel`. Send the client token as for the REST API. |

The backend builds the endpoint URL in the card from the `Host` header of
the request and from `X-Forwarded-Proto` (default `http`).

A task is an investigation. The task ID is the investigation ID. The
context ID is the ID of the investigated run. The task metadata gives the
investigated `unit`.

### Start an investigation

Send `message/send` with a message that has one data part. The data part
has the fields of the REST request: `run_id`, and optionally `unit` and
`sources`. Text parts are ignored.

```sh
curl -sS http://localhost:8080/a2a \
  -H "Authorization: Bearer $SACI_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{
    "jsonrpc": "2.0", "id": 1, "method": "message/send",
    "params": {"message": {
      "kind": "message", "role": "user", "messageId": "failure-run-0f3c9a2b-1.2",
      "parts": [{"kind": "data", "data": {"run_id": "run-0f3c9a2b7d4e4f1a9b8c7d6e5f4a3b2c", "unit": "1.2"}}]
    }}
  }'
```

The answer is the task in state `submitted`. The backend does not wait
for the investigation, also when `configuration.blocking` is true.

- The message ID is the deduplication key. The same message ID returns
  the same task and starts nothing new. The same message ID with another
  data part gives the JSON-RPC error `-32602`. Message IDs and REST
  `Idempotency-Key` values are separate.
- A request for a unit that has a current investigation returns that
  task, as in [one investigation for each unit](#one-investigation-for-each-unit).
- A request that the REST API refuses with `404` or `422` gives a task in
  state `rejected`. The status message gives the reason. A message
  without exactly one data part, or with an invalid data part, is also
  rejected. The backend does not store a rejected task, so `tasks/get`
  does not find it.

### Follow and cancel

`tasks/get` with `{"id": "<task-id>"}` returns the task. The task states
are:

| Investigation | Task state |
| --- | --- |
| `accepted`, `starting` | `submitted` |
| `running`, `stopping` | `working` |
| finished, `concluded` or `inconclusive` | `completed` |
| finished, `error` | `failed` |
| finished, `cancelled` | `canceled` |

A finished task has the run's reason as its status message. When the
agent concluded, the task has two artifacts:

- `result`: a data part with the [result](#read-the-result) of the agent;
- `summary`: a text part with the summary.

`tasks/cancel` with `{"id": "<task-id>"}` requests cancellation, as
`saci cancel` does, and returns the task. The task becomes `canceled` when
the worker stops.

### Errors

| Code | Meaning |
| --- | --- |
| `-32700` | The body is not JSON. |
| `-32600` | The request is not a JSON-RPC 2.0 request with a string or number `id`. |
| `-32601` | The method is not supported, for example `message/stream`. |
| `-32602` | The params are invalid, or a message ID was used with another request. |
| `-32603` | The backend failed. |
| `-32001` | No investigation has this task ID. |
| `-32002` | The investigation already finished, so it cannot be canceled. |

The JSON-RPC answers have HTTP status `200`. A request without a valid
token gets `401` with the REST [error body](api.md#errors).

## The console

In the [web console](web-ui.md), *Investigations* in the menu lists the
investigations of all runs. On a run, the failed unit has an
*Investigation* button. Both open the investigation page at
`/ui/investigations/<investigation-id>`. The page shows the state, the
summary, the hypotheses with their code locations and evidence, the
cloned commits, the bug and all evidence of the agent.
