# Saci tests Saci

The plans in [`plans/`](plans/) are Saci test plans for Saci itself: one
Saci runs them, with a real model, against a second Saci.

| Plan | Checks | Needs the Saci under test |
| --- | --- | --- |
| [`cli-authoring.md`](plans/cli-authoring.md) | `saci --version` and `--help`, `saci new test-plan`, and the plan validation `saci run` does before contacting a backend. | No |
| [`backend-api.md`](plans/backend-api.md) | The REST API with `curl`: health, error bodies, the environment and policy catalog, query and plan validation, idempotent submission, a passing run's events, plan and report, and cancellation. | Yes |
| [`cli-runs.md`](plans/cli-runs.md) | The command line against a backend: passing, failing and cancelled runs, reports, statuses, idempotency and the documented exit codes. | Yes |

The plans run in the Saci worker image, which has `saci`, `curl` and
`jq`, so they test the `saci` built into that image. Each plan works in its
own temporary directory and removes it at the end.

## The Saci under test

[`start-under-test.sh`](start-under-test.sh) starts a backend on
`127.0.0.1:18080` with the walkthrough's scripted model and orders service
([examples/walkthrough](../examples/walkthrough)), so its runs behave the
same way every time and need no API key:

```sh
cargo build --package saci-cli --package saci-testkit
make image
test/start-under-test.sh
```

It stays in the foreground until Ctrl-C. Its workers use the host network,
so it needs Linux.

## Running the plans

In another terminal, start the Saci that runs the plans with
[`saci.yaml`](saci.yaml) (set `llm.model` first) and submit them to its
`saci-under-test` environment:

```sh
export PATH=$PWD/target/debug:$PATH
export SACI_LLM_API_KEY=...        # your model provider's key
(cd test && saci backend start --config saci.yaml) &
saci run test/plans/cli-authoring.md --environment saci-under-test --follow
saci run test/plans/backend-api.md --environment saci-under-test --follow
saci run test/plans/cli-runs.md --environment saci-under-test --follow
```

A coding agent can do these steps with the `saci-run-test-plan` skill
([command line](../docs/cli.md#saci-skill)). Tell it to start the backend
from `test/` with `saci.yaml`, and to use the `saci-under-test`
environment. Start the Saci under test yourself first.

`saci.yaml`'s default policy continues after a failed test, so one failure
does not hide the others. Run the plans one at a time: `backend-api.md`
and `cli-runs.md` both submit runs to the Saci under test and look at its
newest runs.

`cargo test` checks that every plan here stays valid
(`crates/saci-cli/tests/self_test_plans.rs`); it does not run them.

After you change a plan, validate it without running it:

```sh
saci check --strict test/plans/backend-api.md
```
