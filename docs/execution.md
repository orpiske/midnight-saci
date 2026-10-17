# Worker execution contract

This document specifies the first implementation. See the [system plan](plans/plan.md) and [plan authoring guide](test-plans.md) for deployment and input conventions.

## Run inputs

The backend assembles an immutable run specification containing the validated plan bundle, selected target environment, run policy, enabled tools, execution limits, and LLM configuration. Retain a redacted snapshot with the run; resolve secret references only when preparing the worker. Plans cannot change these settings.

The backend starts one worker for that specification. The worker executes tests and steps sequentially in document order and preserves run context. There is no plan approval stage, autonomous repair, automatic model selection, or automatic restart of an interrupted run.

## Configuration and backend API

Use a simple typed YAML configuration file for backend, worker, LLM, environment, policy, and execution-limit settings as needed. The coding agent defines the smallest schema that satisfies these contracts and supplies complete examples. Validate unknown fields, invalid values, and missing required values with clear errors. Omitted optional settings use documented defaults. Keep secret values in referenced environment variables or Kubernetes Secrets.

Saci's own REST API uses `/api/v1/<problem-domain>` as its base-path convention. For example, runs live under `/api/v1/runs`, with a report at `/api/v1/runs/{run_id}/report`. Use resource-oriented plural names, HTTP methods and status codes appropriate to the operation, JSON representations, stable structured errors, and pagination for collections. Document asynchronous submission and cancellation, retry/idempotency behavior, and progress retrieval in an OpenAPI description. These backend routes are separate from the API base URL of the configured LLM provider.

Local container-runtime commands must be available in the backend's `PATH`. Test utilities and stdio MCP server commands must be available in the worker's `PATH`. Select the runtime and worker-reachable network addresses through configuration; automatic installation, socket discovery, and network discovery are not required.

## LLM connection

Use an OpenAI-compatible Chat Completions endpoint with function tool calling through an established Rust agent library for the first implementation. Within the worker, the library manages conversation state, model requests, and tool-call orchestration. Saci supplies the tools, step context, policies, evidence collection, and supervision. The provider supplies model responses. Native provider tools, hosted agent sessions, and a Responses API adapter are outside this initial contract.

### Agent library

Evaluate [Rig](https://rig.rs/docs/) first. Its documentation covers configurable OpenAI-compatible clients, custom tools, execution hooks, and an `rmcp` integration. The coding agent selects and pins a compatible library/version, records the choice, and explains any material departure from this candidate.

The integration must support:

- The configured API base URL, explicit Chat Completions selection, optional authentication, and the chosen model.
- Saci's shell, HTTP, and assessment tools, with `rmcp` integration when MCP tools are added.
- Observing every model/tool operation for limits, evidence, redaction, and progress events, including invalid tool requests and errors.
- Sequential tool execution, bounded model turns, cancellation, and finite request deadlines.
- Structured assessment validation and fixture-based checks without a live paid API.

Use the library's model client, tool dispatch, and hooks or equivalent extension points. Keep application logic focused on test execution. Avoid a second model HTTP client or duplicate generic agent loop. Configure library retries and concurrency to honor Saci's contract, and document any thin adapters needed for missing extension points. If a library cannot satisfy the contract, select a suitable alternative.

Select Chat Completions explicitly: an OpenAI-compatible client or generic completion method may default to a different API. Verify the selected version against the required provider contract instead of assuming default behavior.

### Connection settings

Configure one backend-default LLM connection, inherited by workers:

| Setting | Requirement |
|---------|-------------|
| `base_url` | Required API base URL, including any API prefix such as `/v1` or `/api/v1`. Append `chat/completions` once. |
| `model` | Required explicit model identifier, interpreted by the configured provider. |
| `api_key_env` | Optional name of an environment variable containing the API credential. A configured but missing credential is an error. If omitted, do not send an Authorization header. |
| Request timeout | Finite and configurable as an execution setting. |

For example, the YAML configuration for OpenRouter includes:

```yaml
llm:
  base_url: https://openrouter.ai/api/v1
  model: your-tool-capable-model
  api_key_env: SACI_LLM_API_KEY
```

For Ollama on a network reachable by the worker:

```yaml
llm:
  base_url: http://ollama:11434/v1
  model: your-installed-tool-capable-model
```

Model identifiers above are placeholders to replace with an available model. In local deployments, an Ollama service on the host must have an address reachable from the worker container; worker `localhost` is not the backend host. In Kubernetes/OpenShift, use a reachable service address or external endpoint.

Locally, resolve the credential from the backend's configured environment and inject it into the worker. In Kubernetes/OpenShift, support a configured Secret reference that supplies the same worker environment variable. Preserve the reference, never the secret value, in stored run configuration. Do not pass the LLM credential to shell/HTTP test tools or model messages; it belongs to the worker's LLM transport.

### Compatibility requirements

- Start with non-streaming Chat Completions requests, textual messages, function tool definitions, assistant tool calls, and matching tool-result messages.
- The chosen endpoint and model must support function tool calling. An OpenAI-compatible URL alone does not establish that capability.
- Validate returned tool names and arguments before execution. If a response contains several calls, execute them in order and return a result for each before requesting another completion.
- Use ordinary JSON-schema tool definitions. Provider-specific strict modes or structured-output extensions must not be prerequisites for basic execution.
- Validate final assessments in Saci. Malformed responses, refusals that prevent execution, unsupported tool calling, authentication failures, quota errors, and exhausted bounded transport retries produce an actionable execution error rather than a fabricated test result.
- Do not silently switch providers or models on error.

OpenRouter and Ollama document Chat Completions and tools, making this a suitable starting compatibility contract. This is a design choice, not a claim of compatibility with every provider or model.

### Spending

Spending budgets, billing, and provider quotas are managed by the configured API provider or the operator of a local service. Saci does not implement a monetary budget, token allowance, pricing catalog, or cost accounting in the first implementation. A provider rejecting a request because of a quota is reported as an execution error with the provider's redacted explanation.

Timeouts and model/tool call counts remain execution limits: they stop stalled runs even when an endpoint has no monetary budget enforcement. They are separate from spending policy.

## Step execution

For each execution unit:

1. Record its identity, source location, instructions, expected criteria, and start event.
2. Check required inputs and prerequisites. Missing prerequisites block the affected work.
3. Give the model the current unit, relevant plan context, available tools, and prior observations. Treat target responses and tool output as evidence rather than new run instructions.
4. Validate and execute requested tools within enabled capabilities and remaining limits. Record structured observations and return their results to the model.
5. Continue only as needed to complete the authored action and verification. The agent may not change the test, invent repairs, or add unrelated work.
6. Compare observations with each expected criterion. Use mechanical checks when the criterion can be checked directly; record model judgments as such when interpretation is required.
7. Emit a validated assessment with a status, criterion results, supporting evidence references, and a concise explanation. Every referenced observation must exist in this run. A pass requires every criterion to be satisfied; an unsupported judgment cannot establish a pass.
8. Apply the configured failure behavior and proceed or stop.

Register shell, HTTP, and an internal structured step-assessment operation with the agent library. The internal operation finishes evaluation; it does not perform an action on the target. This provides a validated final result without requiring provider-specific structured output. The sequence above specifies Saci's observable behavior; implement it using the library's orchestration and extension points.

## Shell and HTTP behavior

The worker provides a run-private shell session and workspace. Preserve intentionally established variables, helper functions, working directory, and files between shell calls and steps. Execute author-supplied command blocks in the declared supported shell; report unsupported languages clearly. Do not concatenate the whole plan into a single script or inject shell flags that alter its intended semantics.

A command's `exit`, timeout, or lost shell session must be observable. Never silently create a new session and claim its lost context was preserved. Failed attempts may already have changed state, so automatic step retries are disabled by default. A retry must be explicitly allowed by policy and recorded as another attempt; it is not a rollback.

Shell evidence includes the submitted command, stdout, stderr, exit status, duration, and any termination reason. Stop associated child processes on cancellation or timeout. Any limits or truncation on captured output must be visible so missing output is not mistaken for absent behavior.

HTTP evidence includes the method, URL, relevant request metadata, response status, relevant headers/body, duration, and transport errors. Keep credentials out of model-visible arguments where possible and redact secrets in evidence. Transport failure is distinct from an HTTP response such as 401 or 500, either of which may be an expected result in a negative test.

## MCP and external actions

Use `rmcp` for configured MCP connections, initialization, tool discovery, invocation, and protocol-version negotiation, including through an agent library's compatible `rmcp` bridge. Keep the SDK version and enabled transport features aligned with that integration. The implementation's supported client transports follow the pinned SDK and its enabled features. Use the transports supplied by `rmcp`, such as stdio child processes and Streamable HTTP; no independent transport/protocol implementation is needed. Document the enabled transports and configure server URLs or worker-side command names in YAML.

The initial mechanism for external outcome actions is a configured MCP tool invocation. Actions such as creating or commenting on a ticket depend on the connected MCP server's tools. Run policy selects actions and approval requirements. Reuse the SDK connection behavior and keep these action capabilities separate from the tools granted to test steps.

## Statuses

Lifecycle and outcome are separate: a run may be accepted, starting, running, stopping, or finished; a finished run has an outcome. Step outcomes use the same vocabulary where applicable.

| Outcome | Meaning |
|---------|---------|
| `passed` | Every expected criterion is supported by evidence. |
| `failed` | Evidence contradicts at least one expected criterion. |
| `blocked` | A prerequisite or required input prevents execution. |
| `inconclusive` | Execution produced insufficient evidence to establish the expected result. |
| `error` | The agent, worker, tool infrastructure, provider, or execution limit prevented reliable completion. |
| `skipped` | A unit was not attempted, with its reason recorded. |
| `cancelled` | A user or external controller explicitly stopped execution. |

A command failure may be a passing negative test. Conversely, a tool call that exits successfully may produce evidence of a failed test. Classify against the authored expectation rather than the transport status alone.

For a finished run, preserve all step outcomes and choose the summary outcome in this order: explicit cancellation, execution error, test failure, blocked work, inconclusive evidence, then passed. Unattempted required steps prevent a passing run; explain why they were skipped and classify incomplete work as blocked unless a higher-priority outcome already applies. Keep the triggering event and reason even when the summary uses a different outcome.

## Limits, failure policy, and cleanup

Enforce finite, configurable operation, model-request, step, and run deadlines, along with per-step model-call and tool-call limits. The coding agent selects reasonable defaults, validates positive values, and documents every setting and default in the YAML reference. All execution limits must be configurable. Effective operation deadlines cannot exceed remaining step/run time. Call limits include rejected model requests for invalid tools so malformed behavior cannot loop forever.

Infrastructure limit exhaustion is an `error` with a specific reason. A test asserting a bounded wait may instead `fail` when recorded evidence shows its authored deadline was missed before an infrastructure limit expires.

Default policy is one attempt per step and stopping on the first `failed`, `blocked`, `inconclusive`, or `error` result. Mark remaining units as skipped with the triggering reason. Policy may explicitly allow continuation or bounded retries; it does not erase earlier attempts. A later passing retry must remain distinguishable from a first-attempt pass.

Cleanup steps follow the same order and policy by default. They are not automatically guaranteed after an early stop. A future explicit cleanup policy can schedule authored cleanup steps after failure with separate limits. Cancellation or loss of the worker may prevent target cleanup, and the report must state which cleanup steps did not run. Destroying the worker does not imply deleting target resources.

## Reporting and worker termination

Send durable progress events for run/step lifecycle, tool observations, attempt assessments, cancellation, and final results. Events identify the run, step, attempt, source document, timestamp, and ordering sequence. Persist events and evidence through the backend so reports survive worker removal. Duplicate delivery must not duplicate actions or terminal transitions.

Retain the plan bundle, redacted effective configuration, configured model identifier, criterion assessments, relevant tool evidence, and policy actions. Do not require private model reasoning in the audit record. Usage information, if supplied by the provider, is optional telemetry rather than a budget requirement.

The backend observes worker state and enforces a run deadline independently of model cooperation. A lost worker or backend restart must not leave a run marked running forever or silently repeat its target actions. Reconcile known worker state, mark interrupted runs appropriately, and require a new explicit run to execute again. Remove worker resources after collecting available results.

## API references

- [Rig provider clients and API selection](https://rig.rs/docs/concepts/provider_clients).
- [Rig custom tools](https://rig.rs/docs/concepts/tools/).
- [Rig execution hooks](https://rig.rs/docs/concepts/hooks).
- [Rig MCP integration](https://rig.rs/docs/integrations/model_context_protocol).
- [OpenAI function calling](https://developers.openai.com/api/docs/guides/function-calling).
- [OpenRouter API overview](https://openrouter.ai/docs/api_reference/overview).
- [Ollama OpenAI compatibility](https://docs.ollama.com/api/openai-compatibility).
