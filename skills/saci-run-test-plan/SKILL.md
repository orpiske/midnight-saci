---
name: saci-run-test-plan
description: Run Saci test plans and report the results. Validates each plan with `saci check`, starts the local backend if it is not running (saci-start-backend skill), submits the run, follows it to the end and summarizes the report.
---

# Run a Saci test plan

Use this procedure when the person asks you to run one or more Saci test
plans, for example `test/plans/cli-runs.md`. Saci runs the plan: a worker
and a model do the tests. Your task is to start the run, wait for it, and
tell the person the result. Do not do the tests of the plan yourself.

This skill uses the `saci-start-backend` skill to start the backend. Install
the two skills together, for example with
`saci skill install --all --agent claude`.

For more information, read these documents:

- [docs/cli.md](https://github.com/orpiske/midnight-saci/blob/main/docs/cli.md): the `saci` commands and exit codes.
- [docs/test-plans.md](https://github.com/orpiske/midnight-saci/blob/main/docs/test-plans.md): the plan format.

## Rules

1. **Do not change the plan to get a pass.** If a plan is not valid or a
   test fails, report it. Change a plan only if the person asks you to.
2. **Run one plan at a time.** Plans can share the target environment, and
   one plan can change the result of another plan.
3. **Do not run a plan again automatically.** A second run is a new run.
   Run it again only if the person asks, or if the first run did not start
   (exit code 3 or 4 from `saci run`).
4. **Do not write secrets.** Do not write the value of a token or an API
   key in a file, a command line or the chat. Use the variables that the
   configuration names.

## Procedure

Do these steps for each plan.

### 1. Make sure that `saci` is available

Run `saci --version`. If the command is not found, look for a build in the
current repository, for example `target/debug/saci` or
`target/release/saci`, and add its directory to `PATH`. If you cannot find
it, ask the person to install Saci (`make install`) or to give you the path
of the binary.

### 2. Validate the plan

```sh
saci check plan.md
```

- Exit code 2: the plan is not valid. Report each problem with its file
  and line, and stop. Do not submit the plan.
- Warnings: the plan is valid, but some content does not run (see
  [docs/cli.md](https://github.com/orpiske/midnight-saci/blob/main/docs/cli.md#saci-check)).
  Continue, and include the warnings in your summary.

### 3. Start the backend if it is not running

Follow the `saci-start-backend` skill. Use the configuration file and the
directory that the person or the project documentation gives. For example,
the documentation of a plan directory can tell you to start the backend
with a specified configuration. Continue only when the script exits with
code 0.

If the configuration sets `backend.auth.token_env`, export the token in
`SACI_TOKEN` for the subsequent `saci` commands.

### 4. Select the environment and the policy

Each run needs an environment name. Use the first item of this list that
gives a name:

1. The environment that the person gave.
2. The environment that the plan or its documentation gives, for example
   in the Prerequisites or in a README in the plan directory.
3. The environments that the backend knows:

   ```sh
   curl -fsS "${SACI_BACKEND_URL:-http://127.0.0.1:8080}/api/v1/environments" \
       ${SACI_TOKEN:+-H "Authorization: Bearer $SACI_TOKEN"}
   ```

   If there is only one environment, use it. If there are more, ask the
   person which one to use. Show the names and descriptions.

Use the `default` policy, unless the person gives a different policy.
Policies are in `GET /api/v1/policies`.

### 5. Submit the run

```sh
saci run plan.md --environment <name> [--policy <name>]
```

The command prints the run ID on standard output. Keep the run ID.

- Exit code 2: the backend refused the plan, the environment or the
  policy. Report the message, and stop.
- Exit code 3: the backend does not answer. Do step 3 again, then submit
  again.
- Exit code 4: the reply was lost. Repeat the same command with the
  `--idempotency-key` that the message gives. Do not submit without the
  key: the run can already exist.

### 6. Wait for the end of the run

```sh
saci follow <run-id>
```

The command shows one line for each event and stops at the end of the run.
A run can take many minutes. If your command tool has a time limit, run
the command with a timeout that is shorter than the limit, for example
`timeout 500 saci follow <run-id>`. If the command stops before the end of
the run, the run continues. Then use the number in brackets of the last
event that you saw, and continue:

```sh
saci follow <run-id> --after <seq>
```

`saci status <run-id>` exits with code 8 while the run is not finished.
`timeout` is not on macOS by default. On macOS, use the time limit of your
tool, or `gtimeout` if it is installed.

Do not cancel the run because it is slow. The backend limits the run time.
Cancel a run (`saci cancel <run-id>`) only if the person asks you to.

### 7. Read the report

```sh
saci report <run-id>
```

The exit code of `saci follow` and `saci report` gives the outcome of the
run:

| Code | Outcome | What it means |
| --- | --- | --- |
| 0 | `passed` | All expected results are true. |
| 1 | `failed` | An expected result is false. The target has a problem, or the plan is not correct. |
| 5 | `blocked`, `inconclusive`, or units were skipped | Saci could not find the result of some units. |
| 6 | `error` | Infrastructure, the model provider or a limit stopped the run. The run gives no information about the target. |
| 7 | `cancelled` | A person or a client cancelled the run. |

For an `error` outcome, look for these frequent causes in the report and
in `saci-backend.log`:

- The worker image is missing. With a local runtime, the image is
  `localhost/saci-worker:latest`. Build it with `make image` in the Saci
  repository.
- The model provider refused the request, for example because the API key
  is not set or the model is not correct.

## Summary for the person

Write a short summary for each plan:

- The plan, the environment, the run ID and the outcome.
- The number of units that passed, and each unit that did not pass.
- For each unit that did not pass: its ID and title, the criterion that is
  not true, and the evidence that shows it. Quote the evidence exactly.
- The warnings of `saci check`, if there are warnings.
- If you started the backend: the log file, and how to stop the backend.

Say if a failure is in the target or in the plan only when the evidence
shows it. If you do not know, say so.
