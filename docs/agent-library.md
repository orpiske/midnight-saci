# Agent library decision

**Decision:** Saci uses [Rig](https://rig.rs/) `rig = "=0.44.0"` (the facade
crate over `rig-core`, `rig-agent`, `rig-reqwest` and `rig-rmcp`) for model
interaction, conversation state and tool-call orchestration. Rig was the
first candidate in the plan and satisfies the execution contract with the
small adapters listed below, so no alternative was needed.

The version is pinned exactly because Rig's API changes substantially
between minor releases (0.44 is a redesign; older documentation and examples
do not apply). Upgrades must keep
`crates/saci-worker/tests/rig_compatibility.rs` passing.

Enabled features: `agent`, `reqwest`, `rustls` (and `rmcp` once MCP tools
are added). Default features, derive macros, vector stores and websocket
support are off. Rig's MSRV is 1.95, below Saci's 1.96.

## Contract checks

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

## Thin adapters

These are the only places Saci adds behavior Rig does not provide:

1. **Default turn limit.** Rig's default `max_turns` is 1, which fails any
   run that calls a tool. Saci always sets it from `model_calls_per_step`.
2. **Cancellation token.** Rig has no token API; Saci races the run future
   against its own token with `tokio::select!`, and hooks stop runs from the
   inside.
3. **Bounded transport retries.** Rig has none. `saci_worker::llm::RetryingClient`
   wraps Rig's HTTP transport and repeats a model request that could not
   connect or got HTTP 429 (not an exhausted quota), 500, 502, 503 or 504,
   up to `limits.model_request_retries` times. Tool calls are executed by
   the agent loop only after a successful response, so they are never
   replayed (see [LLM providers](llm-providers.md#retries-and-errors)).
4. **Unknown tool names.** By default Rig fails the run on an unknown tool.
   Saci's hook converts that into a rejected-call observation and feedback
   to the model, counted against the tool-call limit. Rig then also skips
   the valid sibling calls of that assistant message and tells the model so;
   Saci accepts that, because the model is asked to retry anyway.
5. **System prompt shape.** Rig sends the system message as an array of
   content parts, as the OpenAI API allows. This has not been checked
   against every OpenAI-compatible server; one that only accepts string
   content will reject Saci's requests with a provider error.

## How an attempt runs

`saci_worker::agent::Executor` runs one attempt of one unit as one Rig agent
run (`Executor::attempt`):

1. It runs the unit's `run` code blocks (```` ```bash run ````) in order in the
   run's shell session, before any model request. Each counts as a tool
   call and is recorded as evidence. It stops at a block that does not
   exit normally (timeout, cancellation, or a lost or ended session).
2. It builds an agent on the configured chat model with the system prompt
   (`agent::prompt::PREAMBLE`), registers `shell`, `http` and `assess` as
   `DynamicTool`s, and prompts it with the plan context, earlier results,
   environment parameters and secret names, the unit's action, notes on
   code blocks in unsupported languages, the evidence of the `run` blocks
   that already ran, and the numbered expected results. When that evidence
   is enough, the model assesses in its first request; it can still call
   every tool.
3. The `Controls` hook sees every boundary. Before each model request and
   each tool call it checks cancellation and the attempt deadline and counts
   the operation; the first operation over `limits.model_calls_per_step` or
   `limits.tool_calls_per_step` stops the run with `DispatchAction::stop`.
   Unknown tools and malformed arguments arrive in `on_invalid_tool_call`,
   are recorded as rejected evidence and counted too, so repeated invalid
   requests end the attempt. Tool concurrency is fixed at 1, so several
   calls in one reply run in order and each is counted before it runs.
4. A reply without a tool call and without an accepted assessment is sent
   back with `ModelTurnAction::retry_with_feedback`, which consumes the same
   model-request budget. Once `assess` accepts an assessment, `on_outcome`
   stops the run.
5. Cancellation and the attempt deadline cancel an attempt-scoped token
   that the shell and HTTP tools watch. A running tool gets five seconds to
   stop and record its evidence; a pending model request is dropped at once.
   If a shell command is abandoned that way, its session is replaced on the
   next call and that call's evidence says so.
6. The result is the accepted assessment's outcome; otherwise `cancelled`
   when the run was cancelled, or `error` with the limit, deadline,
   provider or tool failure that ended the attempt, or `invalid_assessment`
   when the model stopped without a valid assessment.

The attempt emits `attempt_started`, one `observation` per recorded
evidence, and `attempt_finished`. The hook measures each model request from
its dispatch to its answer, and the recorder adds up the tool durations;
`attempt_finished` reports both as `timing`. Every observation goes through
the run's `Recorder`, which numbers evidence across the run, redacts it with
the run's credentials, and reports it.
