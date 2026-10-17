# Walkthrough: from installation to a report

This walkthrough runs Saci end to end on one Linux machine. It uses a real
worker container, a small orders service as the system under test, and a
scripted model, so it needs no provider account and gives the same
results every time. You will:

1. create a test plan from the template;
2. run it and watch it pass;
3. run a plan that finds a defect and fails;
4. cancel a run in the middle of a slow command;
5. restart the backend and read every report again.

[`examples/walkthrough/walkthrough.sh`](../examples/walkthrough/walkthrough.sh)
does all of this and checks every result. CI runs it on each change (the
`worker-image` job). The sections below explain each step so you can do it
by hand.

## What you need

| Requirement | Why | Check |
| --- | --- | --- |
| Linux, or macOS with Podman machine or Docker Desktop; x86_64 or arm64 | Linux workers share the host's network; on macOS they reach it through the runtime's host alias | |
| Rust toolchain (see `rust-toolchain.toml`) | Builds `saci`, the worker and the fixtures | `cargo --version` |
| Podman (rootless works) or Docker Engine | Runs the worker containers | `podman version` or `docker version` |
| `curl`, `jq` | The script uses them to wait and to read run IDs | `curl --version`, `jq --version` |
| Free loopback ports 18080, 18081 and 11435 | The backend, the orders service and the model | |

You need no credentials: the orders service has no authentication, and
the scripted model needs no API key. The worker image already includes the
utilities the plans use (`sleep`, a POSIX shell). See
[adding utilities](worker.md#adding-utilities) for your own plans.

On macOS (Podman machine or Docker Desktop), containers run in a virtual
machine, so `127.0.0.1` inside a worker is that machine, not your Mac. The
script detects this (`NETWORK=bridge`, the default outside Linux). It drops
`worker.network: host` and points `worker_url`, `llm.base_url` and
`SERVICE_URL` at the runtime's host alias: `host.containers.internal` for
Podman, `host.docker.internal` for Docker. Both runtimes forward the alias
to your Mac's loopback, so the backend, model and orders service stay on
`127.0.0.1`. CI runs the walkthrough on Linux only; the macOS setup is not
tested there. To configure bridged workers by hand, see
[launchers](launchers.md#networking).

## Build

```sh
cargo build --package saci-cli --package saci-testkit
make image                     # localhost/saci-worker:latest
```

This produces three programs:

- `target/debug/saci`, the command line, which also runs the backend;
- `target/debug/saci-fake-target`, the orders service;
- `target/debug/saci-fake-model`, the scripted model.

Run everything at once:

```sh
examples/walkthrough/walkthrough.sh
# with options:
IMAGE=localhost/saci-worker:latest RUNTIME=podman WORK=/tmp/saci-demo examples/walkthrough/walkthrough.sh
```

## The pieces

[`examples/walkthrough`](../examples/walkthrough) contains:

| File | Purpose |
| --- | --- |
| `saci.yaml` | Backend on `127.0.0.1:18080`, workers on the host network, the model at `127.0.0.1:11435`, and one environment `local` with `SERVICE_URL=http://127.0.0.1:18081` |
| `plans/orders.md` | Reads an order (passes), then creates one (fails: the service answers 500) |
| `plans/export.md` | A slow export step (`sleep 300`), the cancellation example |
| `playbook.json` | What the scripted model does for each unit |

**The orders service** (`saci-fake-target --listen 127.0.0.1:18081`) has
one deliberate defect:

- `GET /health` answers 200 `{"status":"ok"}`.
- `GET /orders/1` answers 200 with a shipped order.
- `POST /orders` answers 500 `{"error":"orders database is read-only"}`.

**The scripted model** (`saci-fake-model --listen 127.0.0.1:11435
--playbook playbook.json`) is an OpenAI-compatible Chat Completions
server. It does not reason. For each unit it makes the tool calls the
playbook lists, in order, and then the assessment the playbook gives. It
recognizes the unit from the worker's prompt and the position from the
conversation, so concurrent runs can share it. Every other part of the
run is real: the worker sends the HTTP requests and runs the shell
commands, records and redacts the evidence, checks the assessment against
the plan's expected results, and reports it to the backend. The model
only replaces the provider. A unit the playbook does not cover gets HTTP
500, and the run ends `error` with that reason.

## Step by step

Start the fixtures and the backend in a working directory that holds
`saci.yaml`, `playbook.json` and `plans/`:

```sh
export PATH=$PWD/target/debug:$PATH
cd "$(mktemp -d)" && cp -r ~/src/midnight-saci/examples/walkthrough/* .
saci-fake-target --listen 127.0.0.1:18081 &
saci-fake-model --listen 127.0.0.1:11435 --playbook playbook.json &
saci backend start --config saci.yaml &
export SACI_BACKEND_URL=http://127.0.0.1:18080
```

On macOS, edit the copied `saci.yaml` before starting the backend, so
that workers reach your Mac through the runtime's host alias. With Podman
use `host.containers.internal`; with Docker, `host.docker.internal`:

```yaml
backend:
  worker_url: http://host.containers.internal:18080   # was 127.0.0.1
worker:
  # delete the line `network: host`
llm:
  base_url: http://host.containers.internal:11435/v1  # was 127.0.0.1
environments:
  local:
    parameters:
      SERVICE_URL: http://host.containers.internal:18081   # was 127.0.0.1
```

Keep `listen: 127.0.0.1:18080`, the fixtures' `--listen` addresses and
`SACI_BACKEND_URL` as they are: they are used on your Mac, not inside the
worker.

The backend logs which runtime it found. If the runtime is missing, it
says so, and runs end `error` with that reason (see
[launchers](launchers.md#choosing-the-runtime)).

### 1. Create a plan

```sh
saci new test-plan plans/service-health.md
```

This writes the starter plan "Service health". Its one test runs
`curl -sS -w '\nstatus %{http_code}\n' "$SERVICE_URL/health"` and
expects the output to end with `status 200`. See
[test plans](test-plans.md) for the conventions.

### 2. A passing run

```sh
saci run plans/service-health.md --environment local --follow
```

```text
[1] run started with model walkthrough-playbook
[2] 1.1 attempt 1 started
[3] 1.1 attempt 1   ev-1 shell `curl -sS -w '\nstatus %{http_code}\n' "$SERVICE_URL/health"` exited 0 (12 ms)
[4] 1.1 attempt 1 passed: The service is reachable and reports healthy.
[5] 1.1 passed
[6] run finished passed: all 1 units ran; the summary outcome is passed
```

The command exits 0. If the orders service is not running, curl prints
`status 000`, and the run fails (exit 1): a step passes only on what the
service actually answered.

### 3. A failing run

```sh
saci run plans/orders.md --environment local --follow
```

```text
[3] 1.1 attempt 1   ev-1 shell `curl -sS -w '\nstatus %{http_code}\n' "$SERVICE_URL/orders/1"` exited 0 (11 ms)
[4] 1.1 attempt 1 passed: Order 1 is readable and shipped.
[7] 1.2 attempt 1   ev-2 shell `curl -sS -w '\nstatus %{http_code}\n' -X POST "$SERVICE_URL/orders" \` exited 0 (14 ms)
[8] 1.2 attempt 1 failed: Creating an order fails: the service reports that its database is read-only.
[10] run finished failed: stopped after 1.2 ended failed (policy `default` stops on failure)
```

The command exits 1. The report links each expected result to the evidence
behind it:

```sh
saci report <run id>
```

```text
1.2 Create an order [test]: failed
  attempt 1: failed: Creating an order fails: the service reports that its database is read-only.
    expected result 1: contradicted (mechanical; ev-2): the output ends with status 500, not 201
    expected result 2: contradicted (mechanical; ev-2): the body is {"error":"orders database is read-only"} and has no id
    ev-2 shell `curl -sS -w '\nstatus %{http_code}\n' -X POST "$SERVICE_URL/orders" \` exited 0 (14 ms)
```

### 4. Cancelling a run

```sh
run=$(saci run plans/export.md --environment local)
saci status "$run"            # running (unit 1); exits 8
saci cancel "$run" --wait     # exits 7
```

The worker stops `sleep 300` and keeps its evidence:

```text
1 Run the nightly export [step]: cancelled
  attempt 1: cancelled
    ev-1 shell `sleep 300 && echo exported` exited 137 (2302 ms)
```

See [supervision](supervision.md#cancellation) for what happens when a
worker cannot report its own end.

### 5. Retained reports

Stop the backend (Ctrl-C, or `kill` its process) and start it again with
the same `saci.yaml`. Runs and reports live in `backend.data_dir`
(`./saci-data`, see [operations](operations.md)):

```sh
saci list
```

```text
run-136c…  cancelled     local  Nightly export
run-f35e…  failed        local  Orders
run-8200…  passed        local  Service health
```

`saci report <run id>` prints each report as before, and exits with the
run's outcome code. No worker container is left over: each one is removed
when its run ends. The backend also removes the container of any run that
finished while the backend was stopping
(`podman ps --all --filter label=io.saci.managed=true` lists none).

## Using a real model

To see a real model drive the same plans, keep the orders service and
point `llm` in `saci.yaml` at a tool-capable model. See
[LLM providers](llm-providers.md) for both options:

```yaml
# OpenRouter (export SACI_LLM_API_KEY=... before starting the backend)
llm:
  base_url: https://openrouter.ai/api/v1
  model: your-tool-capable-model
  api_key_env: SACI_LLM_API_KEY
```

```yaml
# Ollama on the host (workers share the host network)
llm:
  base_url: http://127.0.0.1:11434/v1
  model: qwen3:8b
```

A real model decides its own requests and explanations, so the wording
and the evidence numbering differ from the output above. Its verdicts
should still match: the health check and order lookup pass, order
creation fails, and the export is cancelled. These provider smoke tests
are optional and are not part of CI. Model calls on OpenRouter are billed
to your account, and their spending is managed there.

## Troubleshooting

| Symptom | Cause |
| --- | --- |
| `the worker could not be launched: no usable container runtime` | Podman or Docker is not on the backend's `PATH`, or the Docker daemon is not running |
| Run ends `error` with a connection error to `127.0.0.1:18080`, or the worker log repeats `backend unreachable; retrying` | The worker cannot reach the backend. On Linux, check `worker.network: host`. On macOS, use the host alias in `worker_url` (the script does this unless `NETWORK=host`) |
| Run ends `error` with `the playbook has no entry for unit ...` | You changed a unit title; add it to `playbook.json` |
| `address already in use` | Another process uses port 18080, 18081 or 11435; change the ports in `saci.yaml` and on the command lines |
