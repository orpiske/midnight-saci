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
      idempotency_key: request_id   # only if the tool deduplicates on it
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
| `.idempotency_key` | Optional. The tool argument that receives Saci's key for this run and action, `saci-<run-id>-<name>`. Set it only when the tool deduplicates calls on that argument; see [Retries and restarts](#retries-and-restarts). |
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
| `pending_approval` | Recorded with `approval: required`. It is not dispatched until approved. |
| `queued` | Waiting for the dispatcher: automatic, approved, replayed or redispatched. |
| `rejected` | A person rejected it. It is never dispatched. |
| `executing` | Claimed: the claim is stored before the call is made. |
| `completed` | The tool returned a result, or a person reconciled it as completed. |
| `error` | Delivery failed. `error` says why, and `result` holds the tool's text when it answered. |
| `unknown` | The call may have reached the server, but no answer was recorded. It waits for a person. |

The dispatcher connects to the server, initializes it and lists its tools
within `limits.operation_timeout_secs`. It then checks the recorded
arguments against the tool's input schema: required properties, property
types, and `additionalProperties: false`. A mismatch is an `error`, and
nothing is sent. The call itself must answer within the same limit.
Whatever happens to an action, the run's outcome stays as it was.

## Retries and restarts

Saci never retries a call blindly, and promises no exactly-once delivery
for arbitrary tools: only the server can tell whether it acted on a call.
What Saci guarantees:

- **Each action is recorded once.** A run's actions are recorded with its
  final report, once per run and action, however often the worker resends
  its events. Each dispatch first claims the action durably, so two
  dispatchers never send the same attempt.
- **Failures before sending are errors.** When the server cannot be
  reached or the arguments do not fit its schema, nothing was sent, and
  the action ends `error`.
- **Lost answers are unknown.** When the call was sent but no answer came
  (a timeout or a broken connection), or the backend stopped while an
  action was `executing`, the server may or may not have acted.
  - With an `idempotency_key`, the call is replayed with the same key, up
    to 3 attempts in all. The server recognizes the key and does not act
    twice. When attempts run out, the action becomes `unknown`.
  - Without one, the action becomes `unknown` at once and is not sent
    again.
  - A replay that fails before sending anything stays `unknown`, since
    an earlier attempt may have reached the server.

An `unknown` action needs a person who checks the remote system (for
example, whether the ticket exists):

```sh
saci actions show <run-id> open-ticket          # error, attempts and history
saci actions reconcile <run-id> open-ticket --as completed --note "ticket 42 exists"
saci actions reconcile <run-id> open-ticket --as error --note "no ticket"
saci actions redispatch <run-id> open-ticket --note "no ticket; sending again"
```

- `reconcile` records what the remote system shows, `completed` or
  `error`, and sends nothing. Only `unknown` actions can be reconciled.
- `redispatch` queues the same recorded arguments again. It applies to
  `unknown` and `error` actions and names the arguments' digest, like an
  approval. Without an idempotency key, a tool that already acted acts
  again, so check first.
- Anything else is refused with `409 action_not_resolvable`.

### History

Each action keeps its history in SQLite: when it was recorded, approved or
rejected, claimed, answered, failed, found uncertain, replayed,
redispatched or reconciled, with its state and attempt count afterwards
and the error or note. `saci actions show` prints it, and
`GET /api/v1/runs/{run_id}/actions/{name}` returns it.

## Approvals

Approval is configured per action (`approval: required`) and applies after
a run, never before it: runs start immediately as usual. An approval-
required action is recorded as `pending_approval` with its arguments
already rendered. A reviewer decides on exactly that payload:

```sh
saci actions list                                   # what waits for approval
saci actions show <run-id> escalate                 # arguments and digest
saci actions approve <run-id> escalate --digest <digest> --note "on-call agreed"
saci actions reject <run-id> escalate --note "known issue"
```

- Every decision names the SHA-256 digest of the recorded arguments. A
  decision with any other digest is refused (`409 digest_mismatch`), so
  what is sent is what was reviewed. Recorded arguments never change.
- Approval moves the action to `queued`, and the dispatcher sends it.
  Rejection is final.
- Only `pending_approval` actions take a decision. A second decision, or a
  decision on an automatic action, is refused (`409 action_not_pending`).
- The decision, its time and the note are stored with the action in
  SQLite. Pending approvals and decisions survive backend restarts.
- Decisions use the client API, so they need the backend's client token
  when `backend.auth` is configured. Saci records no reviewer identity
  beyond the note; anyone holding the token can decide.

API: `POST /api/v1/runs/{run_id}/actions/{name}/approval` and
`.../rejection` with `{"digest": "...", "note": "..."}`, and
`GET /api/v1/actions?state=pending_approval` across runs
([API](api.md#outcome-actions)).

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

`crates/saci-backend/tests/action_approvals.rs` checks that:

- approval with the reviewed digest sends exactly that payload, and a
  wrong digest changes nothing;
- rejected actions are never sent;
- automatic and already decided actions refuse decisions;
- pending approvals and decisions survive a restart, and only approved
  actions are sent afterwards.

`crates/saci-backend/tests/action_recovery.rs` uses
[`fixtures/recovery.yaml`](../crates/saci-backend/tests/fixtures/recovery.yaml)
and [`fixtures/lost-responses.yaml`](../crates/saci-backend/tests/fixtures/lost-responses.yaml).
The fake server records each call and ignores repeated idempotency keys.
The tests check that:

- events resent after the final report, and two dispatchers at once,
  still record and send each action once;
- after a crash before the call was sent, the keyed action is replayed
  and sent once, and the unkeyed one becomes `unknown`, unsent, until it
  is redispatched;
- after a crash once the server acted, the keyed replay is deduplicated
  by the server, and the unkeyed action becomes `unknown` and is
  reconciled without a second call;
- after a lost answer (a timeout), only the keyed action is replayed;
- the run's outcome stays `failed` throughout.

`crates/saci-cli/tests/action_commands.rs` drives `saci actions list`,
`show`, `approve`, `reject`, `redispatch` and `reconcile` and the actions
section of `saci report`.

`saci-core` unit tests check template rendering and configuration
validation.
