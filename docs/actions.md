# Outcome actions

An outcome action is an MCP tool call the backend makes after a run
finishes, for example to open a ticket when a nightly run fails. The run's
policy selects its actions, and each action names the run outcomes that
trigger it. The tools available depend on the MCP server you connect: a
ticketing server may offer issue creation or commenting. Saci has no
built-in GitHub or Jira client.

Actions follow these rules:

- **After the report is final.** Triggered actions are recorded in the same
  database transaction that stores the final report. An action never exists
  for a report that is not durable, and each action is recorded at most
  once per run.
- **Apart from the verdict.** An action's delivery is stored separately
  from the report, and a failed delivery never changes the run's outcome.
  The CLI shows actions after the report, labelled as such.
- **Apart from workers.** Action servers and their secrets are the
  backend's own. Workers never receive them, and steps cannot call them.
  A plan cannot add, select or change actions.
- **Only the report reaches the tool.** Arguments are rendered from the
  finished, redacted report. Environment parameters, secrets and the run
  specification are not available to them.

## Configuration

```yaml
actions:
  servers:                      # MCP servers the backend calls
    tickets:
      url: http://tickets-mcp:9090/mcp
      headers:
        Authorization: Bearer ${TICKETS_TOKEN}
  secrets:                      # values for ${NAME} in servers, read by the backend
    TICKETS_TOKEN: { env: SACI_TICKETS_TOKEN }
  definitions:
    open-ticket:
      server: tickets
      tool: create_issue
      on: [failed, error]       # run outcomes that trigger it
      approval: none            # or required
      arguments:
        title: "{{plan.title}}: {{run.outcome}}"
        body: |
          Run {{run.id}} in {{environment}} ended {{run.outcome}} ({{run.counts}}).

          {{failures}}

          Evidence:
          {{evidence}}

policies:
  nightly:
    on_failure: continue
    actions: [open-ticket]
```

| Setting | Meaning |
| --- | --- |
| `actions.servers.<name>` | An MCP server, written like [`mcp_servers`](mcp.md#configuration): `command`, `args` and `env` for stdio, or `url` and `headers` for Streamable HTTP. |
| `actions.secrets.<NAME>` | `{ env: VAR }`: a backend environment variable. `${NAME}` in `servers` expands to it, and it is redacted from stored results and errors. |
| `actions.definitions.<name>.server` | A server under `actions.servers`. |
| `.tool` | The tool to call. |
| `.on` | Run outcomes that trigger the action: `passed`, `failed`, `blocked`, `inconclusive`, `error`, `cancelled`. Required. |
| `.approval` | `none` (default): dispatched once recorded. `required`: kept as `pending_approval` until approved. |
| `.arguments` | The tool's arguments. Strings may contain the variables below. Other values are sent as written. |
| `policies.<name>.actions` | The actions considered when a run under this policy finishes. |

The configuration is checked at startup: unknown servers, unknown
variables, empty `on` lists and undefined policy actions are errors. A run
keeps the action definitions of its policy as they were when it was
accepted. Server connection settings are read when the action is
dispatched.

### Variables

| Variable | Value |
| --- | --- |
| `{{run.id}}` | The run ID. |
| `{{run.outcome}}`, `{{run.reason}}` | The run's outcome and why it ended. |
| `{{run.counts}}` | Unit counts, such as `1 failed, 2 skipped`. |
| `{{run.created_at}}`, `{{run.finished_at}}` | RFC 3339 timestamps. |
| `{{plan.title}}`, `{{plan.entry}}` | The plan title and entry document. |
| `{{environment}}`, `{{policy}}`, `{{model}}` | Names recorded in the report header. |
| `{{summary}}` | One line per finished unit: outcome, ID, title and the assessment's explanation. |
| `{{failures}}` | The unsuccessful units, each with its criterion results. |
| `{{evidence}}` | Excerpts of the evidence cited by unsuccessful units: commands with exit codes and output, HTTP requests with status and body, MCP results. Each output is cut to 1000 bytes, and the whole to 16 KiB. |

Evidence is redacted with the run's secrets before it is stored, so
excerpts never contain secret values.

### Where the server must be reachable

The dispatcher runs inside the backend process, not in a worker:

- A stdio server's `command` must be installed where the backend runs: on
  its `PATH`, or as an absolute path. The process starts with only `PATH`
  and its configured `env` entries.
- An HTTP server's `url` must be reachable from the backend: from the
  backend's host locally, or from the backend pod in Kubernetes.
- With Helm, set action servers under `config.actions`, and pass secrets
  to the backend through `backend.extraEnv`, for example with
  `valueFrom.secretKeyRef`. The backend does not read Kubernetes Secrets
  itself, so `{ secret, key }` references are rejected here.

## Delivery

| State | Meaning |
| --- | --- |
| `pending_approval` | Recorded with `approval: required`. It is not dispatched. |
| `queued` | Waiting for the dispatcher. |
| `executing` | Claimed: the claim is stored before the call is made. |
| `completed` | The tool returned a result. |
| `error` | Delivery failed. `error` says why, and `result` holds the tool's text when it answered. |

The dispatcher connects to the server, initializes it and lists its tools
within `limits.operation_timeout_secs`. It then checks the recorded
arguments against the tool's input schema: required properties, property
types, and `additionalProperties: false`. A mismatch is an `error`, and
nothing is sent. The call itself must answer within the same limit. An
action is dispatched once. It is not retried automatically, and the run's
outcome stays as it was whatever happens.

When the backend stops while an action is `executing`, the server may or
may not have received the call. On restart such an action becomes `error`
with that explanation, and it is not called again.

## Inspecting actions

`saci report <run>` lists the actions after the report:

```text
Actions (delivery does not change the outcome):
  open-ticket: completed (tickets/create_issue): created issue 42
```

`GET /api/v1/runs/{run_id}/actions` returns them with the exact arguments
and their SHA-256 digest ([API](api.md#outcome-actions)).

## Checked by

`crates/saci-backend/tests/outcome_actions.rs` replays recorded runs
through the worker API, and the backend dispatches to a stdio
`saci-fake-mcp`. The tests check the following:

- the server receives exactly the recorded arguments, rendered from the
  report, with no environment parameters or secret values;
- the server process gets only `PATH` and its configured variables;
- tool errors and schema mismatches end in `error`, and the run outcome
  and report are unchanged;
- approval-required actions stay `pending_approval`;
- only the outcomes an action names trigger it.

`saci-core` unit tests check template rendering and configuration
validation.
