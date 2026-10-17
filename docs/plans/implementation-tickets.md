# Implementation ticket drafts

The reviewed breakdown is published as 12 parent issues and 33 bounded task issues. See the [created issue index](issue-index.md) for the assignable tasks and their GitHub links. The sections below retain the feature-group requirements; they are not single implementation assignments.

These are local drafts for the maintainer to turn into issues. Numbers below identify dependencies within this document; they are not GitHub issue numbers. The [system plan](plan.md), [authoring guide](../test-plans.md), and [execution contract](../execution.md) define the intended behavior.

## Delivery milestones

1. **Local execution:** tickets 1–9 deliver plan creation, a self-hosted backend, isolated workers, a configurable OpenAI-compatible LLM connection, and a CLI that runs plans and retrieves evidence.
2. **Namespace deployment:** ticket 10 delivers the same workflow on Kubernetes/OpenShift with little supporting infrastructure.
3. **Extensions:** tickets 11–12 add configured MCP servers and additional outcome actions. Keep their extension boundaries in the initial contracts, without requiring their implementations for local execution.

## 1. Establish the Rust project and quality gates

Create the initial Rust project with CLI, backend, and worker entry points and shared contract types. Keep the module layout small and document the chosen boundaries. Favor the Rust ecosystem used in [wisdom-maker](https://github.com/orpiske/wisdom-maker/blob/main/Cargo.toml) and [Wanaku](https://github.com/wanaku-ai/wanaku/blob/main/Cargo.toml). Outcomes, simplicity, and code quality take priority over a fixed dependency list.

Acceptance criteria:

- The project builds reproducibly from a fresh checkout. Use Rust edition 2024 and pin a supported toolchain; keep the declared minimum Rust version and Clippy MSRV consistent with the selected dependencies and toolchain. The coding agent selects compatible versions and a small package/workspace layout.
- Prefer `tokio` for asynchronous execution, `clap` for CLI parsing, `serde`/`serde_json` for data, `reqwest` for asynchronous HTTP, `tracing`/`tracing-subscriber` for diagnostics, and `thiserror`/`anyhow` for appropriate error boundaries. `rusqlite` is the preferred starting point for SQLite; keep blocking database work off async executor threads.
- Use `rmcp` for MCP integration. For YAML, prefer the maintained Serde-compatible implementation used by Wanaku (`yaml_serde`, imported there as `serde_yaml`). Add other dependencies only where the implementation requires them; explain material departures from these preferences.
- Require an established Rust agent library for model interaction and tool-call orchestration. Evaluate Rig first; ticket 4 selects and records the library/version after checking the execution contract. Keep its `rmcp` dependency compatible with the project's MCP integration.
- Adopt Wanaku's `clippy.toml` together with its Rust/Clippy lint enablement from `Cargo.toml`. Ensure every workspace crate inherits the lint configuration. Preserve the safety, correctness, clarity, complexity, async, and disallowed-method rules and document any necessary scoped exceptions.
- CI enforces formatting and Clippy with warnings treated as errors; relevant behavioral checks run as implementation appears.
- Contributors can find build, lint, and local development instructions in the repository.

Dependencies: none.

## 2. Define run, environment, policy, event, and report contracts

Implement versioned serializable types for run input, plan units, target configuration, worker settings, attempts, evidence, progress, and final outcomes. Keep lifecycle distinct from outcome.

Acceptance criteria:

- Types represent the required configuration and all statuses described in the execution contract.
- Use a simple typed YAML configuration file. Document sections, defaults, required fields, secret references, and examples for local and Kubernetes/OpenShift use. Keep the schema small and avoid a configuration expression language.
- The coding agent selects and documents reasonable finite defaults for operation/model-request/step/run deadlines and model/tool call limits. Every execution limit must be configurable; omitted settings use documented defaults. Spending budgets and token allowances are not required settings.
- Policy defaults to one attempt and stopping on the first unsuccessful unit; any continuation or bounded retry is explicit.
- Evidence references, run/step/attempt identities, event ordering, source locations, and redacted configuration snapshots have stable representations.
- Unknown or unsupported settings and invalid limits produce clear errors. Secret values are excluded from serializable public records.
- Define REST resource paths under `/api/v1/<problem-domain>`, for example `/api/v1/runs` and `/api/v1/runs/{run_id}/report`. Use plural resource names, appropriate HTTP methods/status codes, JSON representations, stable error responses, and pagination for collections. Document asynchronous submission/cancellation semantics, retry/idempotency behavior, and the worker event interface. Publish an OpenAPI description of the implemented API.

Dependencies: 1.

## 3. Parse, validate, and scaffold Markdown test plans

Implement the authoring conventions and `saci new test-plan <file.md>`.

Acceptance criteria:

- Parse GFM into phases, tests/steps, actions, verification instructions, expected criteria, and source locations in document order.
- Report malformed structure, duplicate IDs, missing actions, and missing expected results with actionable locations.
- Bundle explicitly invoked relative procedures, rejecting missing files, escape paths, and recursive inclusion. Preserve the submitted source documents.
- Distinguish explanatory links and content from invoked procedures and executable units. Specify the accepted procedure-invocation syntax in the guide rather than relying on unconstrained model inference.
- `saci new test-plan saci-on-the-cloud.md` works offline with an embedded template, derives its title from the filename, and refuses to overwrite a file.
- The template, guide, CLI help, and parser agree, with representative valid and invalid examples for the parser.

Dependencies: 2.

## 4. Integrate a Rust agent library and OpenAI-compatible LLM connection

Use an established Rust agent library to provide the LLM connection, conversation handling, and tool-call orchestration described in the execution contract. Evaluate Rig first; choose a suitable alternative if required and document the reason. Saci implements application-specific test semantics through the library's extension points.

Acceptance criteria:

- Configure base URL, explicit model, optional API credential reference, and a finite request timeout through YAML.
- Record the selected library/version and demonstrate its support for sequential tool calls, operation hooks, cancellation, configurable model-turn limits, and fixture-based execution. Enable only the features required by this application.
- Reuse the library's provider client and orchestration. Keep any missing-capability adapters small and documented; do not implement a duplicate model HTTP client or generic agent loop.
- Use non-streaming Chat Completions with function tools and correct tool-call/result correlation. Preserve required assistant message fields when continuing the conversation.
- Select the Chat Completions API explicitly when the library's generic completion client defaults to a different API. Configure library retry/concurrency behavior to match Saci's limits and sequential execution.
- Support API prefixes and optional Authorization without duplicating endpoint path components.
- Document OpenRouter and worker-reachable Ollama configurations; require an appropriate tool-capable model.
- Handle malformed responses, unsupported tool calling, refusal, timeout, authentication, rate limiting, and quota errors explicitly. Any transport retries are bounded and do not replay target tool calls.
- No monetary budget, pricing service, or token allowance is required. Model/tool call limits remain independent execution controls.
- Validate transport behavior using a controlled API fixture rather than requiring paid provider access in routine checks. Document an optional real-provider compatibility smoke procedure.

Dependencies: 2.

## 5. Implement worker shell and HTTP tools

Provide the initial tool implementations inside the worker image.

Acceptance criteria:

- Shell calls preserve variables, helper functions, working directory, and files within a run; document the supported shell languages.
- Capture commands, stdout/stderr, exit codes, duration, and process termination. An explicit shell `exit` or lost session is reported without silently losing context.
- HTTP calls capture response status, relevant headers/body, duration, and transport errors separately.
- Operation deadlines and cancellation stop subprocess trees and in-flight requests; output capture is bounded with visible truncation.
- Tool schemas and arguments are validated, unknown tools are rejected, and only configured capabilities are exposed.
- LLM transport credentials are excluded from the shell environment and target HTTP credentials. Evidence is redacted before storage and before being sent to the model.
- Include an isolated OCI worker image suitable for both local execution and Kubernetes/OpenShift, with a documented way to supply required test utilities.

Dependencies: 2.

## 6. Implement sequential agent execution and evidence-based assessment

Connect the parsed plan and tools using the selected agent library's orchestration, hooks, and tool interfaces. Implement Saci's step ordering, evaluation, policies, and evidence around that integration.

Acceptance criteria:

- Execute authored units sequentially, preserving run context and refusing autonomous repairs or unrelated work.
- Register shell, HTTP, and step-assessment tools with the library. Integrate limit enforcement, progress events, and evidence capture at its model/tool boundaries; verify it cannot bypass these controls through hidden retries or parallel dispatch.
- Evaluate every expected criterion and attach existing evidence references to a validated final assessment.
- Distinguish failed expectations, blocked prerequisites, inconclusive evidence, and execution errors. A printed `PASS` or zero exit alone cannot establish all expectations.
- Record whether each assessment used direct mechanical evidence or model interpretation. Reject nonexistent evidence references and incomplete passing assessments.
- Enforce all execution limits independently of model instructions, including repeated invalid tool requests.
- Default to stopping; record remaining units as skipped. Preserve earlier attempts when an explicit retry or continuation policy is used.
- Cancellation halts execution. Cleanup follows the documented initial policy, and reports identify teardown that did not run.
- Exercise the loop against scripted model/tool fixtures covering a successful plan, a failed negative test, provider failure, invalid arguments, missing evidence, timeout, and preserved shell state.

Dependencies: 3, 4, 5.

## 7. Implement the backend API and durable run records

Build a single backend service with SQLite persistence for submissions, events, evidence, and reports.

Acceptance criteria:

- API supports submission, listing/inspection, progress retrieval, report retrieval, and cancellation under `/api/v1/<problem-domain>`, following the REST conventions and OpenAPI contract from ticket 2.
- Validate and store a bundle and redacted effective configuration before worker launch. A valid run begins without an approval stage.
- Persist per-step/attempt results and ordered progress events; reconnecting clients can retrieve missed events.
- Reports preserve outcomes, evidence, policy actions, and unattempted cleanup after the worker is removed.
- Duplicate events cannot repeat terminal transitions or target actions.
- Use authenticated backend access when exposed beyond loopback, with a simple documented token setup for the first version. Worker reporting uses separate run-scoped credentials.
- Reconcile interrupted runs after backend restarts without silently rerunning target actions.

Dependencies: 2, 3.

## 8. Launch and supervise local container workers

Connect the backend and agent through a local worker launcher. Support Podman and Docker through their command-line binaries resolved using the backend process's `PATH`. Keep discovery limited to checking those executable names, with an explicit runtime setting and a documented deterministic preference when both are available.

Acceptance criteria:

- A valid submission creates exactly one isolated container worker using the configured image.
- Resolve the selected runtime executable from the backend's `PATH` and report missing or unusable commands clearly. Do not require socket discovery, installation-directory scanning, or automatic installation.
- Pass the immutable specification, run-private workspace, required secret references/values, and authenticated reporting configuration without exposing unrelated backend credentials or host directories.
- Document networking for worker-to-target, worker-to-backend, and worker-to-LLM access, including a host-local Ollama service.
- Configure worker-reachable addresses and any necessary runtime network options explicitly in YAML. The coding agent documents a working setup for each supported runtime; automatic host/network discovery is not required. Required test and stdio MCP binaries must likewise be in the worker's `PATH`.
- The backend observes startup and termination, enforces a run deadline independently, and reports launch failures or lost workers as execution errors.
- Cancellation stops the worker and available evidence is retained before worker resources are removed.
- Local setup requires the backend, its data directory, and a supported container runtime; it does not require a separate database or queue service.

Dependencies: 6, 7.

## 9. Deliver the CLI for complete local and remote runs

Add CLI commands to launch/configure a local backend, submit a plan bundle, follow progress, inspect a report, and cancel a run. Keep the offline authoring command from ticket 3.

Acceptance criteria:

- One documented walkthrough creates a plan, supplies environment/LLM configuration, submits it, and retrieves evidence from a local backend.
- The same run commands work against a remotely configured backend URL and credential.
- Progress shows the current unit, attempts, and results. Reports distinguish failure, incomplete work, cancellation, and infrastructure errors.
- Commands provide documented exit codes suitable for automation and a machine-readable report option.
- Include a complete example plan and environment configuration that use a disposable controlled target, without embedded credentials.
- Publish local installation and troubleshooting instructions, including how to add required CLI utilities to a worker image.

Dependencies: 3, 8.

## 10. Add namespace-scoped Kubernetes/OpenShift execution and packaging

Implement a Kubernetes Job launcher and a simple installation package, using the same backend, worker image, and contracts as local execution. Recommend Helm for the initial package.

Acceptance criteria:

- Install the backend, Service, service accounts, namespace-scoped RBAC, configuration/Secret references, and PVC into one existing namespace/project without application-specific cluster-wide resources or privileged containers.
- Run a single backend replica with SQLite on suitable storage. Support the cluster's storage class or an existing PVC; document storage requirements and upgrade behavior that avoids simultaneous SQLite writers.
- Create and observe one Job per run; supervise deadlines, failure, cancellation, result collection, and resource removal.
- Keep worker identity separate from the backend's launcher identity; worker access to target resources is explicitly configured for the test environment.
- Support ordinary OpenShift user-ID restrictions and Kubernetes non-root execution; no container runtime socket is required in cluster mode.
- Provide a Kubernetes access example and an OpenShift access example, keeping external exposure optional through port forwarding.
- Demonstrate a plan against a target in the same namespace/project, using an external or reachable local-network OpenAI-compatible provider and configured credentials.

Dependencies: 8, 9.

## 11. Extend worker tools through configured MCP servers

Add MCP tool discovery and invocation to the worker's existing tool interface. Keep shell and HTTP available without an MCP dependency in a run configuration.

Acceptance criteria:

- Environment configuration declares enabled MCP servers, credentials, and tool selection; plan content cannot introduce servers or grant capabilities.
- Use `rmcp` client support for transport handling, initialization, and protocol-version negotiation. Enable the client transports needed for configured servers from those provided by the pinned SDK (such as stdio child processes and Streamable HTTP), and document the enabled feature set. Compatibility follows the selected `rmcp` version; custom transports or protocol-version implementations are not required.
- Reuse the chosen agent library's `rmcp` bridge when available, aligning SDK versions and capabilities. Keep Saci's policy, observation, and limit controls around MCP calls just as for built-in tools.
- Configure connections in YAML. Resolve stdio server commands through the worker's `PATH`; use configured URLs for HTTP servers.
- Translate discovered tools into the LLM's tool definitions with stable names that do not collide with built-in or assessment tools.
- MCP calls participate in the same deadlines, call limits, cancellation, redaction, evidence, and reporting contracts.
- Distinguish MCP connection/tool errors from tested target behavior. Document a complete example integration.

Dependencies: 6, 9.

## 12. Add configurable outcome actions and bounded extension hooks

Extend run policies with explicit follow-up actions after execution. Use configured MCP tool invocations through `rmcp` as the initial external-action mechanism. Available actions depend on the connected server's tools; for example, a ticketing MCP server may expose issue creation or commenting. The coding agent documents the YAML action schema and approval flow as part of this ticket.

Acceptance criteria:

- Actions are explicitly configured, receive the recorded outcome and evidence, and run under separately scoped capabilities.
- Policy selects the MCP server/tool and the conditions for invoking it. Reuse the configured `rmcp` connection behavior and validate arguments against discovered tool schemas; a provider-specific ticketing client is not required for the initial mechanism.
- Distinguish test outcome from action delivery success; a ticketing failure cannot erase or change a test verdict.
- Avoid duplicate external actions across retries/restarts using durable delivery state and suitable idempotency behavior.
- Support configurable human approval for selected actions without adding pre-run approval to ordinary execution.
- Keep repair/patch generation outside the test worker's responsibilities.
- If cleanup-after-failure is added, specify its explicit plan marker, scheduling rules, separate limits, and cancellation behavior before changing the authoring guide and parser.

Dependencies: 7, 9, 11.

## Implementation guidance

- **Tickets 1 and 4:** use the preferred library ecosystem and Wanaku lint baseline above. The coding agent chooses compatible versions, a supported toolchain, a minimal layout, and an established Rust agent library. Evaluate Rig first and document the selected integration.
- **Ticket 2:** use simple YAML and REST under `/api/v1/<problem-domain>`. The coding agent chooses and documents configurable defaults and the concrete schema/API operations within these rules.
- **Ticket 8:** use runtime binaries in the backend's `PATH` and explicitly configured networking. Worker-side executables must be in the worker's `PATH`.
- **Tickets 11–12:** use the pinned `rmcp` client's supported transports and negotiation. External actions invoke tools from configured MCP servers; the coding agent defines and documents action configuration and approvals.

These guidelines leave implementation details to the coding agent while preserving the agreed plan format, code-quality baseline, and provider-managed spending policy.
