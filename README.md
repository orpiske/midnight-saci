# Midnight Saci

**An AI test engineer that runs your Markdown test plans and shows its work.**

You write a test plan the way you would for a colleague: steps, commands
and what you expect to see. Saci carries it out against a real
environment, step by step, with an LLM deciding what to do and a small,
bounded set of tools doing it. Every verdict points at the evidence behind
it, so you can check the reasoning instead of taking it on faith.

Saci follows the plan. It does not invent repair steps or patch your code.

## What a run looks like

```text
$ saci run plans/orders.md --environment local --follow
[3] 1.1 attempt 1   ev-1 shell `curl -sS -w '\nstatus %{http_code}\n' "$SERVICE_URL/orders/1"` exited 0 (11 ms)
[4] 1.1 attempt 1 passed: Order 1 is readable and shipped.
[7] 1.2 attempt 1   ev-2 shell `curl -sS -w '\nstatus %{http_code}\n' -X POST "$SERVICE_URL/orders" \` exited 0 (14 ms)
[8] 1.2 attempt 1 failed: Creating an order fails: the service reports that its database is read-only.
[10] run finished failed: stopped after 1.2 ended failed (policy `default` stops on failure)
```

And the report ties each expected result to what was observed:

```text
$ saci report <run id>
1.2 Create an order [test]: failed
  attempt 1: failed: Creating an order fails: the service reports that its database is read-only.
    expected result 1: contradicted (mechanical; ev-2): the output ends with status 500, not 201
    expected result 2: contradicted (mechanical; ev-2): the body is {"error":"orders database is read-only"} and has no id
```

## What you get

- **Plans in plain Markdown.** Phases, tests, steps and expected results,
  with no YAML front matter. `saci new test-plan` gives you a starter.
- **One isolated worker per run**, as a Podman or Docker container or a
  Kubernetes Job.
- **Bounded tools:** shell, HTTP and MCP, with credentials redacted from
  the evidence.
- **Any OpenAI-compatible model**, such as one on OpenRouter or a local
  Ollama.
- **Runs you can trust later:** retries and stop policies, cancellation,
  deadlines, and reports that survive a backend restart.
- **Outcome actions:** after a run, call MCP tools (open a ticket, post to
  chat), optionally waiting for your approval. Actions never change the
  verdict.

## Try it in a few minutes

The walkthrough runs everything on one machine with a small orders service
and a scripted model, so you need no API key or provider account. You need
a Rust toolchain, Podman or Docker, `curl` and `jq`.

```sh
git clone https://github.com/orpiske/midnight-saci.git
cd midnight-saci
cargo build --package saci-cli --package saci-testkit
make image
examples/walkthrough/walkthrough.sh
```

It writes a plan, watches one run pass and one fail, cancels a slow run,
restarts the backend and reads every report back. The
[walkthrough guide](docs/walkthrough.md) explains each step so you can do
it by hand, and shows how to switch to a real model.

To install the `saci` binaries in `/usr/local/bin`, run `make install`. To
install them in a different directory, set `BINDIR`, for example
`make install BINDIR=$HOME/bin`. See [installing](docs/cli.md#installing).

## Your first plan

```sh
saci new test-plan plans/service-health.md
```

A plan reads like this:

````markdown
# Test Plan: Orders

## Phase 1: Orders API

### Test 1.1: Read an existing order

Read order 1 and print the response status:

```bash
curl -sS -w '\nstatus %{http_code}\n' "$SERVICE_URL/orders/1"
```

**Expected result:**

- The output ends with `status 200`.
- The order's `status` is `shipped`.
````

To validate it offline, without a backend, run
`saci check plans/service-health.md`. To let your coding agent help, for
example to convert existing plans or to run them, install the Saci skills with
`saci skill install --all --agent claude` (or `codex`, `cursor`,
`copilot`, `gemini`, `opencode`). Then start a backend and submit it (the build puts `saci` in
`target/debug`):

```sh
export PATH=$PWD/target/debug:$PATH
cp examples/config/local.yaml saci.yaml   # then edit it for your system
export SACI_LLM_API_KEY=...                # your model provider's key
saci backend start --config saci.yaml &
saci run plans/service-health.md --environment local --follow
```

[`examples/config/local.yaml`](examples/config/local.yaml) is a starting
configuration: set `llm.model`, and the environment's parameters and
secrets for the system you are testing. See [writing test plans](docs/test-plans.md) for the full
format and the [command line](docs/cli.md) for every command and exit
code.

## How it works

The `saci` command sends your plan to the backend, which stores the run in
SQLite and starts a worker for it. The worker's agent works through the
plan with the LLM and its tools, and reports events and evidence back with
a token valid only for that run. You follow progress and read the report
through the CLI, the [web console](docs/web-ui.md) or the
[REST API](docs/api.md).

[Architecture](docs/architecture.md) has the diagram and the role of each
crate.

## Documentation

**Start here**

- [Walkthrough: from installation to a report](docs/walkthrough.md)
- [Writing test plans](docs/test-plans.md), with a
  [starter template](docs/templates/test-plan.md) and a
  [complete example](docs/templates/example.md)
- [Command line](docs/cli.md)
- [Web console: runs, live progress, evidence and action review in the browser](docs/web-ui.md)
- [Configuration reference](docs/configuration.md)

**Running tests**

- [Worker execution and LLM configuration](docs/execution.md)
- [LLM providers: OpenRouter, Ollama, retries and errors](docs/llm-providers.md)
- [Worker tools: shell, HTTP and step assessment](docs/tools.md)
- [MCP servers](docs/mcp.md)
- [MCP script runner: scripts as MCP tools](docs/script-runner.md)
- [Julia 1 judge: a local System One judge](docs/julia-judge.md)
- [Run sequencing, retries, stopping and cleanup](docs/sequencing.md)
- [Outcome actions and approvals](docs/actions.md)
- [Failure investigations](docs/investigations.md)
- [Running plans when code changes: `saci watch`](docs/watch.md)
- [Credentials, evidence redaction and isolation limits](docs/security.md)

**Deploying and operating**

- [The worker image and adding utilities](docs/worker.md)
- [Launching workers with Podman or Docker](docs/launchers.md)
- [Supervising runs: cancellation, deadlines and recovery](docs/supervision.md)
- [Operating the backend: data directory and records](docs/operations.md)
- [Running workers as Kubernetes Jobs](docs/kubernetes.md)
- [Installing with Helm on Kubernetes or OpenShift](docs/helm.md)
- [Namespace workflows](docs/namespace-workflows.md)

**Reference and design**

- [Architecture](docs/architecture.md)
- [REST API and worker protocol](docs/api.md) ([OpenAPI](docs/openapi.yaml))
- [Run contracts: identities, statuses, events and reports](docs/contracts.md)
- [Agent library decision (Rig)](docs/agent-library.md)
- [Brand guide](artwork/BRAND.md)
- [System plan](docs/plans/plan.md),
  [implementation tickets](docs/plans/implementation-tickets.md) and
  [issue index](docs/plans/issue-index.md)

## Contributing

Saci is written in Rust. `make check` runs the same formatting, Clippy and
test gates as CI. [Development](docs/development.md) covers the layout and
the quality gates.

## About the name

The Saci is a trickster from Brazilian folklore who slips into houses at
night and gets into everything. This one does its rounds on your systems
instead, and leaves a report behind.

## License

Apache-2.0.
