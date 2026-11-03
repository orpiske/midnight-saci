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
[quickstart](docs/get-started/quickstart.md) explains each step so you can
do it by hand. To install the binaries, see
[install Saci](docs/get-started/install.md).

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
`saci check plans/service-health.md`. To run it with your own model
provider, see [run your first plan with a real model](docs/get-started/run-with-a-real-model.md).
To let your coding agent convert or run plans, install the Saci skills
([use Saci from a coding agent](docs/run-tests/use-agent-skills.md)).

## How it works

The `saci` command sends your plan to the backend, which stores the run in
SQLite and starts a worker for it. The worker's agent works through the
plan with the LLM and its tools, and reports events and evidence back with
a token valid only for that run. You follow progress and read the report
through the CLI, the [web console](docs/run-tests/web-console.md) or the
[REST API](docs/reference/rest-api.md).

[Architecture](docs/get-started/architecture.md) has the diagram and how a
run flows.

## Documentation

The [documentation home](docs/index.md) has a "Choose your path" table.

- [Get started](docs/get-started/index.md): quickstart, installation and
  architecture.
- [Write test plans](docs/write-plans/index.md): write, reuse and convert
  plans.
- [Run tests](docs/run-tests/index.md): run plans, follow them, use the web
  console and investigate failures.
- [Connect Saci to other systems](docs/connect/index.md): MCP tools,
  outcome actions, `saci watch`, A2A and the script runner.
- [Operate Saci](docs/operate/index.md): model providers, workers,
  Kubernetes and Helm, data and credentials.
- [Reference](docs/reference/index.md): commands, configuration, plan
  format, REST API and contracts.
- [Contribute to Saci](docs/contributing/index.md): development, testing and
  design notes.

## Contributing

Saci is written in Rust. `make check` runs the same formatting, Clippy and
test gates as CI. [Development](docs/contributing/development.md) covers the layout and
the quality gates.

## About the name

The Saci is a trickster from Brazilian folklore who slips into houses at
night and gets into everything. This one does its rounds on your systems
instead, and leaves a report behind.

## License

Apache-2.0.
