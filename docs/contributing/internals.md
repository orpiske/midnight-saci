# Saci internals

This page is for contributors. It records how Saci uses its main libraries
and how the backend and the script runner are built inside. For the crate
layout, see [development](development.md#workspace-layout).

## Agent library

**Decision:** Saci uses [Rig](https://rig.rs/) `rig = "=0.44.0"` (the facade
crate over `rig-core`, `rig-agent`, `rig-reqwest` and `rig-rmcp`) for model
interaction, conversation state and tool-call orchestration. Rig was the
first candidate in the plan and satisfies the [execution contract](design/execution-contract.md) with the
small adapters listed below, so no alternative was needed.

The version is pinned exactly because Rig's API changes substantially
between minor releases (0.44 is a redesign; older documentation and examples
do not apply). Upgrades must keep
`crates/saci-worker/tests/rig_compatibility.rs` passing.

Enabled features: `agent`, `reqwest`, `rustls`, `rmcp` and `typesafeai`
(the System One judge). Default features, derive macros, vector stores and
websocket support are off. Rig's MSRV is 1.95, below Saci's 1.96.

### Contract checks

`crates/saci-worker/tests/rig_compatibility.rs` runs Rig against
`saci-testkit`'s scripted OpenAI-compatible server and verifies:

| Requirement | How Rig meets it |
|-------------|------------------|
| Custom OpenAI-compatible base URL | `OpenAIConfig::with_base_url`. The request URL is the base with trailing `/` removed plus `/chat/completions`; Rig never inserts `/v1`, so prefixes such as `/v1` and `/api/v1` come from configuration. |
| Explicit Chat Completions | `client.chat(model)` always targets `/chat/completions`. (`client.completion(model)` with the default OpenAI dialect targets the Responses API, so Saci never uses it.) Saci uses a `Dialect::gateway`, whose default route is Chat Completions as well. |
| Optional authentication | `Auth::OptionalBearer` with an empty key sends no `Authorization` header; a key sends `Bearer <key>`. |
| Tool exchange | Runtime-defined `DynamicTool`s with JSON-schema parameters. Assistant `tool_calls` and the `role: tool` results carry matching `tool_call_id`s. |
| Observation hooks | `AgentHook::on_dispatch` sees every model request and tool call (name, call ID, raw arguments); `on_outcome` sees every model response and tool result; `on_invalid_tool_call` sees unknown tools and malformed arguments. |
| Sequential tools | Tool concurrency defaults to 1: calls in one assistant message run one at a time, in order. Saci never raises it. |
| Limits | `max_turns(n)` bounds model calls per run (`PromptError::MaxTurns`). Hooks enforce tool-call limits by returning `DispatchAction::stop`, which ends the run (`PromptError::Cancelled`) without another model request. |
| Cancellation | Dropping the run future cancels the in-flight request and tool; Saci selects on its own cancellation token. |
| Deadlines | The `reqwest` client timeout bounds each model request; `tokio::time::timeout` bounds attempts and runs. |
| No hidden retries | Rig does not retry provider requests: a 429, 500, timeout or malformed body produces exactly one request. Provider status, body and a `retryable` hint are available on the error. |
| Fixture-based testing | `saci-testkit::FakeModel` checks the real wire format without a paid API. |
| MCP | `rig-rmcp` turns `rmcp` client tools into Rig tools and depends on `rmcp` 2.x (2.2). Saci therefore uses `rmcp` 2.x through Rig's re-export rather than the newer 3.x line, so there is a single SDK version. |

### Thin adapters

These are the only places Saci adds behavior Rig does not provide:

1. **Default turn limit.** Rig's default `max_turns` is 1, which fails any
   run that calls a tool. Saci always sets it from `model_calls_per_step`.
2. **Cancellation token.** Rig has no token API; Saci races the run future
   against its own token with `tokio::select!`, and hooks stop runs from the
   inside.
3. **Bounded transport retries.** Rig has none. `saci_worker::llm::RetryingClient`
   wraps Rig's HTTP transport and repeats a model request that could not
   connect, timed out, or got HTTP 429 (not an exhausted quota), 500, 502,
   503 or 504, up to `limits.model_request_retries` times. Tool calls are executed by
   the agent loop only after a successful response, so they are never
   replayed (see [LLM providers](../operate/llm-providers.md#retries-and-errors)).
4. **Unknown tool names.** By default Rig fails the run on an unknown tool.
   Saci's hook converts that into a rejected-call observation and feedback
   to the model, counted against the tool-call limit. Rig then also skips
   the valid sibling calls of that assistant message and tells the model so;
   Saci accepts that, because the model is asked to retry anyway.
5. **System prompt shape.** Rig sends the system message as an array of
   content parts, as the OpenAI API allows. This has not been checked
   against every OpenAI-compatible server; one that only accepts string
   content will reject Saci's requests with a provider error.


## MCP SDK

Saci uses the [`rmcp`](https://crates.io/crates/rmcp) SDK 2.2 for transports,
initialization and protocol-version negotiation; the protocol versions it
supports are those of that release. It is the same major version Rig 0.44's
`rmcp` bridge (`rig-rmcp`) depends on, and Cargo resolves both to one copy;
upgrading one requires upgrading the other.

The connection code lives in the `saci-mcp` crate. The worker uses it for
step tools, and the backend for [outcome actions](../connect/outcome-actions.md), each with
its own servers and credentials.

Enabled `rmcp` features:

| Feature | Provides |
|---------|----------|
| `client` | The MCP client role. |
| `transport-child-process` | Stdio servers started as child processes. |
| `transport-streamable-http-client-reqwest`, `reqwest` | Streamable HTTP servers, over rustls. |

The legacy HTTP+SSE transport, WebSocket and OAuth flows are not enabled.

## Backend API implementation

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
top of the worker's own redaction, in every free-text field: evidence,
assessment and criterion explanations, error messages, and skip and run-end
reasons. Long-polling clients are woken as soon as
the batch is committed.

`saci_worker::report` is the worker side: `fetch_specification` retries
while the backend is unreachable; `deliver` reads the run's event channel,
sends batches of up to 100 events and 16 MiB of JSON
(`saci_core::api::MAX_EVENT_BATCH_BYTES`, the body limit of the events
route), drops what the backend acknowledges,
resends the rest (from the acknowledged sequence on a `sequence_gap`), and
returns once the channel is closed and every event is acknowledged;
`watch_cancellation` polls `control` and cancels the run's token. Delivery
retries an unreachable or failing (5xx) backend with backoff for up to five
minutes, then the worker reports itself lost; a refusal (401, 403, 409
`run_finished`, 422) ends delivery at once. An observation too large for
one request on its own has its captured output halved until it fits; the
cut is recorded as truncation, with the original size kept.

## Script runner

The code is in `crates/saci-script-runner`:

| Module | Owns |
|--------|------|
| `config` | YAML types and startup checks. |
| `registry` | The fixed tool list, input schemas and argument validation. |
| `executor` | Process execution, deadlines, output limits and process group cleanup. It does not use MCP. |
| `server` | The `rmcp` handler, the HTTP router, the token and `Origin` checks, and audit events. |

The runner uses `rmcp` 2.2 (the workspace version) in stateful Streamable
HTTP mode. Stateful mode is necessary because a cancellation notification
must reach the session that runs the call. The tests in
`crates/saci-script-runner/tests` start the server on a local port and use
the `rmcp` client over HTTP.

## Next steps

- [Development](development.md): build, layout and quality gates.
- [Testing](testing.md): what each test suite checks.
- [Design notes](design/index.md): the reasons behind each feature.
