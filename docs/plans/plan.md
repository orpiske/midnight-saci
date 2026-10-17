# AI-Based Test Engineer: General Plan

## Goal

Build a simple AI-based test engineer that executes human-authored Markdown test plans against configured environments. The system follows the plan and records evidence. It can change test resources when instructed by the plan, but does not invent repair steps or patch application code.

The design is guided by three principles:

- **Simplicity:** one plan format and one run lifecycle across clients.
- **Trustworthiness:** limited capabilities, explicit policies, and an auditable run record.
- **Adaptability:** clients and tools can be extended without changing the core workflow.

## Technical baseline

- Implement the system in Rust.
- Prefer the library ecosystem used by [wisdom-maker](https://github.com/orpiske/wisdom-maker/blob/main/Cargo.toml) and [Wanaku](https://github.com/wanaku-ai/wanaku/blob/main/Cargo.toml), including Tokio and `rmcp`. The coding agent chooses compatible versions and a minimal layout; simplicity and code quality take priority over specific dependency choices.
- Use an established Rust agent library for model interaction, conversation handling, and tool-call orchestration. [Rig](https://rig.rs/docs/) is the first candidate to evaluate; the coding agent selects a library that satisfies the execution contract and documents the choice. Saci owns plan ordering, policy, evidence, and worker supervision, integrating them through the library's extension points.
- Use the Wanaku Rust project's [`clippy.toml`](https://github.com/wanaku-ai/wanaku/blob/main/clippy.toml) and Cargo Rust/Clippy lint enablement, inherited by every workspace crate. Keep Clippy warnings enabled as errors in CI. Use edition 2024 and a supported pinned toolchain with consistent MSRV declarations.
- Use a simple YAML configuration file. Every execution limit is configurable; the coding agent selects and documents reasonable defaults.
- Define the REST API under `/api/v1/<problem-domain>`, using standard resource naming, HTTP semantics, JSON, documented errors, and an OpenAPI contract.
- Keep the initial deployment self-contained: one backend service, one worker image, and a local SQLite database. Avoid requiring an external database or cluster-wide services for the initial experience.

## System shape

```text
Web / desktop / TUI clients
             |
             v
      Backend API and coordinator
             |
             v
    Short-lived per-run worker
             |
             v
  Configured target environment
```

### Clients

Clients submit runs and display progress, results, and evidence. The first client can be simple; the backend API should allow web, desktop, and TUI clients to be added over time.

### Backend

The backend validates run requests, stores run history, applies outcome policies, starts workers in its own infrastructure, and collects worker events and results.

### Worker

The backend starts one short-lived worker per run. A worker receives the run specification and only the tools and secrets required for that run. It executes the plan, records its actions and observations, reports results to the backend, and is removed after completion.

The target environment must be reachable from the backend's infrastructure. This is part of environment setup.

The backend uses a worker-launcher boundary so the same worker image can run in both deployment modes:

- **Local:** start an isolated worker container through Podman or Docker binaries available in the backend's `PATH`. Required test utilities and stdio MCP binaries must be available in the worker's `PATH`. Configure runtime selection and reachable network addresses explicitly.
- **Kubernetes/OpenShift:** create a short-lived Job in the application's namespace/project. The install should need only namespace-scoped permissions to create and observe those Jobs.

The initial Kubernetes deployment is single-replica and stores run data in SQLite on a PersistentVolumeClaim. This avoids requiring a separately provisioned database while retaining run history across pod restarts. Scaling the backend beyond one replica or adding a shared database is a later design decision.

## Inputs

Each run is defined by three inputs:

1. **Test plan:** Markdown containing ordered steps. Each step has an action and an expected result. Actions use natural language and are carried out by the AI with the available tools.
2. **Environment:** Connection and execution configuration for the target system.
3. **Run policy:** Configurable behavior for success and failure.

### Lightweight plan structure

Plans use GitHub Flavored Markdown with conventions modeled on Wanaku: phases, numbered tests or steps, prose and fenced commands, and explicit expected results. The [authoring guide](../test-plans.md) defines the conventions and reusable procedure bundles. For example:

```markdown
# Test Plan: Example workflow

## Phase 1: Service availability

### Test 1.1: Check service health

Send an HTTP GET request to the service health endpoint.

**Expected result:**

- The response status is 200.
```

The CLI will provide `saci new test-plan <file.md>` to generate an editable plan from a bundled [starter template](../templates/test-plan.md). This command works without a backend or LLM and refuses to overwrite existing files. The guide, template, CLI help, and parser must stay aligned.

Plans cannot add tools or MCP servers to a run. Structure is parsed deterministically; the agent interprets actions and evaluates expected results within the parsed execution units. Run context must preserve variables, helper functions, working directories, and files needed by subsequent steps. These conventions follow Wanaku's style; existing plans may need explicit expected results or execution headings added.

### Environment

An environment may contain:

- Endpoints, hostnames, addresses, and other connection settings.
- References to credentials and secrets. Secret values are injected at run time and must not be written to plans or exposed to the model unnecessarily.
- Enabled MCP servers and their configuration.
- Shared, non-secret parameters used by plans.
- Execution settings such as step/run timeouts and worker resource limits.

Stable values belong in the environment; test-specific values can be supplied with the plan or run request.

### LLM connection

Workers use an OpenAI-compatible Chat Completions API with function tool calling through the selected Rust agent library. Configure a base URL, explicit tool-capable model, and optional API credential reference. This supports a common connection contract for providers such as OpenRouter and a reachable local Ollama service. The initial implementation uses one backend-default connection inherited by workers. Library defaults must preserve the selected API, execution limits, and sequential tool behavior.

Spending budgets and quotas are managed by the API provider or local service operator. Saci does not require monetary budgets, token allowances, or cost accounting. Timeouts and model/tool call limits remain runtime controls to stop stalled execution. See the [execution contract](../execution.md) for settings and error behavior.

### Run policy

Run policy specifies what happens after success or failure. It may configure actions such as stopping, retrying, requesting human approval, or taking a follow-up action (for example, creating or updating a ticket). The default run is one pass through the plan. Any retries or external actions must be explicitly configured.

Keep policy separate from the environment: the environment describes how to connect and execute; policy describes how to respond to outcomes.

## Tools

The worker has these built-in capabilities:

- **Shell** for command-line actions.
- **HTTP** for requests to services and APIs.

Additional capabilities are provided through configured MCP servers, such as browser automation or specialized integrations for databases and other systems. Use `rmcp` and its supported client transports and protocol-version negotiation. Available external outcome actions are configured MCP tool invocations, subject to run policy and approval settings. The run record should identify which tools were called and retain their relevant results. MCP servers are configured for an environment or run; a plan cannot introduce an arbitrary server.

## Run lifecycle

1. A client submits a plan, environment selection, and run policy.
2. The backend validates the request and creates a run record.
3. The backend immediately starts a short-lived worker in its own infrastructure.
4. The worker executes plan steps once, using only its enabled tools and recording actions, observations, evidence, and step outcomes.
5. The backend applies the configured outcome policy and stores the final report.
6. The client displays progress and the report. The worker is removed; the run record remains available for review.

The system does not require a pre-run approval. Human approval can be configured as a policy action when appropriate.

## Run record and report

For each run, retain at least:

- Plan, environment, and policy identifiers or snapshots sufficient to understand the run later.
- Run status and timestamps.
- Per-step status, action taken, observed result, and evidence.
- Tool calls and relevant outputs, with secrets redacted.
- Outcome-policy actions and their results.
- Final summary of passed, failed, skipped, or blocked steps.

## Extension points

MCP is the tool extension point for the initial design. Hooks or plugins may later customize behavior around execution or outcomes, but they are not required for the default workflow and should not complicate the first implementation.

## Initial scope

- Submit and execute a structured Markdown plan immediately.
- Run each plan once by default.
- Start an isolated, short-lived worker from the backend for each run.
- Support shell and HTTP directly, with configured MCP servers for additional tools.
- Configure target connection details, secret references, parameters, timeouts, and worker limits per environment.
- Configure outcome behavior separately from environment settings.
- Keep an auditable run record and report.
- Keep clients decoupled from execution so multiple client types can use the backend.
- Support both local self-hosted execution and a simple single-namespace Kubernetes/OpenShift deployment.
- Use the same isolated worker image in local containers and Kubernetes Jobs.
- Keep initial state storage self-contained with SQLite; Kubernetes deployment uses a PVC.
- Apply the Wanaku Clippy configuration and enforce Clippy in CI.
- Connect to a configurable OpenAI-compatible API, including OpenRouter or a worker-reachable Ollama endpoint, using a tool-capable model.
- Leave spending budgets to the provider while enforcing execution deadlines and model/tool call limits.

## Execution contract and ticket drafts

The [worker execution contract](../execution.md) defines sequential execution, preserved shell context, expected-result evaluation, evidence, statuses, runtime limits, failure policy, cleanup, and LLM settings. The [implementation ticket drafts](implementation-tickets.md) break this design into ordered tasks and acceptance criteria for the maintainer to create as issues.

## Details delegated to implementation

- Choose a small Rust layout, compatible dependency versions, toolchain, and an established Rust agent library within the preferred ecosystem and Wanaku lint baseline.
- Define the concrete YAML schema, REST operations under `/api/v1/<problem-domain>`, configurable execution defaults, and detailed parser diagnostics.
- Implement explicit local runtime selection using binaries in `PATH`, document networking, and define worker isolation and namespace-scoped RBAC.
- Use `rmcp` for supported MCP transports and version negotiation; document enabled features, credential injection, and configured outcome-action approvals.
- Deliver the CLI/local workflow first, followed by Kubernetes/OpenShift deployment, as described in the ticket drafts.

## Suggested implementation sequence

1. **Rust project foundation:** workspace, backend and worker entry points, Wanaku Clippy configuration, formatting/lint CI.
2. **Contracts and authoring:** implement the documented Markdown conventions and validation, plan bundles, `saci new test-plan <file.md>`, run request, environment, policy, step events, and report types. Deliver the authoring guide, bundled template, and examples together.
3. **Worker execution:** agent-library integration, worker image and shell/HTTP tools, timeouts, cancellation, and structured evidence.
4. **Local vertical slice:** backend API, SQLite run history, local container worker launcher, and CLI submit/follow/report commands.
5. **Kubernetes/OpenShift deployment:** namespace-scoped Job launcher, RBAC, single-replica backend, SQLite PVC, and install instructions.
6. **MCP and outcome extensions:** configured MCP servers and additional policy actions after the base run path is working.
