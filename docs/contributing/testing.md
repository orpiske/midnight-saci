# Testing

This page is for contributors. It lists the test gates, the fixtures that
replace real models and services, and what the main test suites check.

## Run the tests

```sh
make check         # cargo fmt --check, clippy -D warnings, cargo test
make web-check     # web console checks, browser tests, backend console tests
make image-check   # build the worker image and run a plan in it
```

CI (`.github/workflows/ci.yml`) runs the same gates, and also the
`worker-image` and `kubernetes` jobs below. See
[quality gates](development.md#quality-gates).

## Fixtures

The `saci-testkit` crate has controlled fixtures, so tests need no real
model or service: a scripted OpenAI-compatible model (`saci-fake-model`), a
fake orders service (`saci-fake-target`), a fake MCP server and a fake
Kubernetes API. Use them instead of real models or services.

### The fake MCP server

`saci-fake-mcp` (in `crates/saci-testkit`) is a controlled MCP server with
`echo`, `fail`, `sleep` and `env` tools, and `record`, which stands in for an
external action: it appends each call to the file named by
`FAKE_MCP_RECORD`. It speaks stdio by default, or
Streamable HTTP with `saci-fake-mcp --http 127.0.0.1:9090` (endpoint `/mcp`,
requiring `Authorization: Bearer $FAKE_MCP_TOKEN` when that variable is set).

## Saci tests Saci

[`test/`](../../test/README.md) holds Saci test plans for Saci itself: one
Saci runs them, with a real model, against a second Saci.

## Outcome actions

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
[`fixtures/recovery.yaml`](../../crates/saci-backend/tests/fixtures/recovery.yaml)
and [`fixtures/lost-responses.yaml`](../../crates/saci-backend/tests/fixtures/lost-responses.yaml).
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

## Supervision

- `crates/saci-backend/tests/supervision.rs` uses a fake runtime to cover
  uncertain launches, cancellation, deadlines, exits with and without a
  report, disappearance, cleanup, each recovery case and leftover removal.
- `crates/saci-backend/tests/container_launcher.rs` uses Docker or Podman
  and the real image (`SACI_WORKER_IMAGE`). It launches a worker that runs
  a plan to the end, and cancels one in the middle of `sleep 60`, which
  ends `cancelled` with the step's evidence. Both containers are removed.

## MCP tools in steps

`crates/saci-worker/tests/mcp_tools.rs` runs whole plans through the
`saci-worker` binary with the fake server. It checks the report, the
redacted evidence, the offered tool names and schemas, tool and protocol
errors, a server that cannot start, and runs without MCP.

## Kubernetes Jobs

- `crates/saci-backend/tests/kubernetes_launcher.rs` runs the supervisor
  against an in-memory Kubernetes API
  (`saci_testkit::fake_kubernetes`). It covers the Job and Secret shape,
  identities, Secret references, removal, cancellation, failure with
  logs, restart recovery, a repeated launch, and API refusals.
- The CI `kubernetes` job creates a kind cluster and loads the image. A
  backend outside the cluster, using `kubectl proxy`, runs a plan to a
  pass in a real Job, then cancels a second run in the middle of
  `sleep 120`. Both Jobs are removed.

## Helm chart

- `crates/saci-backend/tests/helm_chart.rs` renders the chart with Helm
  and checks the following:
  - the generated `saci.yaml` loads as a valid configuration;
  - nothing cluster-wide is created;
  - the Role grants no wildcard verbs;
  - one non-root `Recreate` replica runs on the claim;
  - with the OpenShift API present, no `fsGroup` is set;
  - existing claims and Secrets are used;
  - values reach the configuration.
- The CI `kubernetes` job installs the chart on kind with the built image
  and checks the API through `kubectl port-forward`. It then upgrades the
  release and checks that one backend pod remains and the token is
  unchanged.
- The same job runs the [namespace workflow](../operate/namespace-workflow.md):
  runs, a backend restart, cancellation, retained reports and Job removal.
  That page also lists the platforms tested so far. OpenShift is not among
  them.

## Worker images in CI

To enable publication, configure the repository secrets `QUAY_USERNAME`
and `QUAY_PASSWORD` with credentials that can push to the Quay repository.
The optional repository variables `CONTAINER_REGISTRY` and
`CONTAINER_GROUP` override `quay.io` and the GitHub repository owner.
Without both secrets, CI builds and tests the images but skips publication.
Pull requests also build and test both architectures without publication.
Architecture images use tags unique to the workflow run. The manifest job
combines these images only after all CI jobs pass.

- **Checked in CI.** The `worker-image` job builds the image on native
  x86_64 and AArch64 runners and runs a
  fixture-backed plan in it as UID 123456, against a scripted model.

## Next steps

- [Development](development.md): build the workspace and add a crate or a
  skill.
- [Tested platforms](../operate/namespace-workflow.md#tested-platforms):
  the platforms that the Kubernetes workflow ran on.
