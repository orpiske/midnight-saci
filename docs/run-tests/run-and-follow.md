# Run and follow plans

This page tells you how to submit a plan to a backend, follow the run, read
its report and cancel it. All options and exit codes are in the
[command line reference](../reference/cli.md).

## Prerequisites

- A running backend (`saci backend start --config saci.yaml`, see
  [`saci backend start`](../reference/cli.md#saci-backend-start)).
- `SACI_BACKEND_URL` set to the backend URL, or `--backend` on each
  command. When the backend requires a client token, export it in
  `SACI_TOKEN` (see [connecting to a backend](../reference/cli.md#connecting-to-a-backend)).
- A valid plan (`saci check plan.md`).

## Submit a run

```sh
saci run plan.md --environment staging --follow
```

`--environment` names an environment of the backend configuration. Add
`--policy <name>` to use a run policy other than `default`. Without
`--follow`, the command prints the run ID and returns when the backend
accepts the run.

If the connection breaks after the request was sent, the command exits 4
and prints the idempotency key that it used. Repeat the command with
`--idempotency-key <key>`: the backend returns the run that it already
accepted and does not start a second one (see
[when the reply is lost](../reference/cli.md#when-the-reply-is-lost)).

## Follow a run and read the report

```sh
saci list                      # recent runs, newest first
saci status <run-id>           # state, progress and outcome
saci follow <run-id>           # progress until the run finishes
saci report <run-id>           # units, attempts, criteria and evidence
```

`saci follow` reconnects by itself when the connection fails, and continues
after the last event that it printed. Ctrl-C stops `follow`, not the run.
`saci report` ties each expected result to the evidence behind it. The
exit code of these commands is the outcome of the run, for example 0 for
`passed` and 1 for `failed` (see [exit codes](../reference/cli.md#exit-codes)).

To see the same information in a browser, open the
[web console](web-console.md).

## Cancel a run

```sh
saci cancel <run-id> --wait
```

The worker stops after its current action, skips the remaining units and
runs the remaining cleanup units (see [cleanup](how-a-run-executes.md#cleanup)).
With `--wait`, the command waits until the run has stopped and exits with
its outcome, normally 7 (`cancelled`).

## Next steps

- [How a run executes](how-a-run-executes.md): attempts, retries, stops and
  cleanup.
- [Investigate failures](investigate-failures.md): find the probable cause
  of a failed unit in the source code.
- [Outcome actions](../connect/outcome-actions.md): report results to other
  systems after a run.
