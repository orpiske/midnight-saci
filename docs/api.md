# REST API and worker reporting protocol

The backend's REST API is described by [`openapi.yaml`](openapi.yaml). Every
route lives under `/api/v1/<problem-domain>`: client routes under
`/api/v1/runs`, worker routes under `/api/v1/worker/runs`. Bodies are JSON.
Example payloads in [`api-examples/`](api-examples/) are checked against the
Rust types in `saci_core::api` by the test suite.

## Authentication

- **Clients** send `Authorization: Bearer <token>`, where the token is the
  value of the environment variable named by `backend.auth.token_env`. When
  `backend.auth` is not configured the backend only accepts the loopback
  listener (configuration validation enforces this) and requires no token.
- **Workers** send a run-scoped bearer token that the backend generates when
  it launches the worker and stores only as a hash. It grants access to the
  worker routes of that one run; requests for any other run get `403
  forbidden`, and it is rejected on client routes.
- `GET /api/v1/health` never requires a token.

Token comparison is constant-time. A backend whose `backend.auth.token_env`
names an unset or empty variable refuses to start.

## Errors

Errors use one body shape and stable codes:

```json
{ "error": { "code": "validation_failed", "message": "the plan is invalid", "details": ["plan.md:5: ..."] } }
```

| Code | Status | Meaning |
|------|--------|---------|
| `bad_request` | 400 | Malformed JSON, query or header. |
| `unauthorized` | 401 | Missing or wrong token. |
| `forbidden` | 403 | A worker token used for another run. |
| `not_found` | 404 | No such run or evidence. |
| `idempotency_conflict` | 409 | An `Idempotency-Key` reused with a different body. |
| `run_finished` | 409 | Cancellation or worker reporting after the run finished. |
| `sequence_gap` | 409 | A worker batch skipped a sequence number. |
| `action_not_pending` | 409 | The action is not awaiting approval: it was already decided, or never needed approval. |
| `digest_mismatch` | 409 | An approval or rejection named a digest other than the recorded arguments'. |
| `validation_failed` | 422 | Invalid plan bundle, unknown environment or policy; `details` lists every diagnostic with its file and line. |
| `internal` | 500 | The backend failed. |

## Submitting a run

`POST /api/v1/runs` with a [`SubmitRun`](api-examples/submit-run.json) body:
the bundle (entry document plus every invoked procedure, keyed by
bundle-relative path), an environment name and an optional policy name.

The backend validates the bundle and the names, stores the run with a
redacted snapshot of its effective configuration, and answers `202 Accepted`
with a [`RunSummary`](api-examples/run-summary.json) and a `Location`
header. The worker launch happens afterwards, asynchronously; there is no
approval stage. Invalid input is rejected with `422` before anything is
stored or launched.

A body that is not valid JSON, or has fields other than `bundle`,
`environment` and `policy`, is a `400 bad_request`; a well-formed body whose
plan, environment or policy is invalid is a `422 validation_failed`.

**Retries and idempotency.** A submission is safe to retry only with an
`Idempotency-Key` header. Repeating the request with the same key and an
identical body returns the original run with `200 OK` and starts nothing
new; the same key with a different body fails with `409
idempotency_conflict`. Without a key, a client that lost the response cannot
tell whether the run was created and should list runs before resubmitting.

## Listing and inspecting

- `GET /api/v1/runs?limit=50&cursor=...` lists runs newest first. Pass
  `next_cursor` from one page as `cursor` to get the next; the last page has
  no `next_cursor`.
- `GET /api/v1/runs/{run_id}` returns the summary: lifecycle, outcome once
  finished, current unit, counts and the latest event sequence.
- `GET /api/v1/runs/{run_id}/report` returns the full report. It is final
  once `lifecycle` is `finished` and remains available after the worker is
  removed.
- `GET /api/v1/runs/{run_id}/evidence/{evidence_id}` returns one observation.

Query parameters are validated: `limit` must be 1 to 200 (default 50),
`wait` 0 to 60, and a `cursor` must come from an earlier page; anything else
is a `400 bad_request`. A malformed run or evidence ID is a `404`.

## Following progress

`GET /api/v1/runs/{run_id}/events?after=<seq>&wait=<secs>` returns events
with `seq > after` in order ([example](api-examples/event-page.json)). With
`wait`, the backend holds the request until a new event exists, the run
finishes, or the wait (at most 60 seconds) ends. A client keeps
`next_after` and reconnects with it, so a dropped connection never loses or
repeats an event. `finished: true` means no more events will appear.

## Cancelling

`POST /api/v1/runs/{run_id}/cancellation` records the request (`202`) and
the backend tells the worker to stop. Repeating it is harmless and returns
the same `cancel_requested_at`. Cancellation is *requested* until the run's
lifecycle reaches `finished`; the outcome is then `cancelled` unless the run
had already ended for another reason. Cancelling a finished run returns
`409 run_finished`.

## Outcome actions

`GET /api/v1/runs/{run_id}/actions` lists the run's outcome actions with
their state (`pending_approval`, `queued`, `executing`, `rejected`,
`completed`, `error`), the exact arguments, their digest, any decision, the
attempts, and the result or error. The list is empty until the run
finishes, and when no action was triggered. Delivery never changes the
run's outcome. See [outcome actions](actions.md).

`GET /api/v1/actions?state=<state>&limit=<n>` lists actions in one state
across runs, oldest first. The default state is `pending_approval`.

`POST /api/v1/runs/{run_id}/actions/{name}/approval` and
`POST /api/v1/runs/{run_id}/actions/{name}/rejection` decide on an
action waiting for approval. The body is
`{"digest": "<sha256 of the recorded arguments>", "note": "optional"}`.
They answer `200` with the action, `404 not_found` without such an action,
`409 digest_mismatch` for another digest, and `409 action_not_pending`
when the action is not waiting for approval.

## Worker reporting protocol

A worker is started with the backend URL, its run ID and its run-scoped
token. It then:

1. **Fetches its input** with `GET /api/v1/worker/runs/{run_id}/specification`
   and validates it before doing anything else. The specification contains
   secret references only; secret values are injected into the worker's
   process environment by the launcher.
2. **Reports events** with `POST /api/v1/worker/runs/{run_id}/events` and a
   batch of consecutive events ([example](api-examples/event-batch.json)).
   The backend persists the batch in one transaction and only then answers
   with the highest persisted sequence number
   ([`EventAck`](api-examples/event-ack.json)). Events at or below that
   number are ignored, so a worker that did not receive an acknowledgment
   resends the same batch; deduplication guarantees each event is recorded
   once, not that target actions happened once. A batch that would skip a
   number fails with `409 sequence_gap` and the acknowledged sequence in
   `details` ([example](api-examples/error-sequence-gap.json)); the worker
   resends from there. Events after `run_finished` are rejected with `409
   run_finished`.
3. **Polls control** with `GET /api/v1/worker/runs/{run_id}/control` and
   stops when `cancel_requested` is true, reporting `cancellation_requested`
   and finally `run_finished` with outcome `cancelled`.

Evidence travels inside `observation` events, already redacted, so it stays
readable from the backend after the worker disconnects or is removed.

## Implementation

`saci_backend::api::router` serves these routes with axum. A submission is
validated and stored in one step (`api::submit::prepare`, then
`Store::create_run`); only a newly created run is handed to the
`Supervisor`, which starts its worker. A cancellation request is recorded
first and then handed to the supervisor. Long polls wait on a change signal
that the backend raises whenever it stores events or changes a run, then
re-read the database, so a reply always reflects persisted state.

### Worker credentials and the worker client

`saci_backend::api::issue_token` creates a worker token (256 random bits as
hex) when a worker is launched and stores only its SHA-256 hash. Worker
routes look the hash up: an unknown or missing token is `401`, a token for
another run is `403`. The client token never matches a worker hash, so it
cannot use worker routes either.

Before storing a batch, the backend also redacts the secret values it can
read itself (environment secrets with an `env` reference, the LLM key), on
top of the worker's own redaction. Long-polling clients are woken as soon as
the batch is committed.

`saci_worker::report` is the worker side: `fetch_specification` retries
while the backend is unreachable; `deliver` reads the run's event channel,
sends batches of up to 100 events, drops what the backend acknowledges,
resends the rest (from the acknowledged sequence on a `sequence_gap`), and
returns once the channel is closed and every event is acknowledged;
`watch_cancellation` polls `control` and cancels the run's token. Delivery
retries an unreachable or failing (5xx) backend with backoff for up to five
minutes, then the worker reports itself lost; a refusal (401, 403, 409
`run_finished`, 422) ends delivery at once.
