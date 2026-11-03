# Worker reference

A worker executes exactly one run. The backend's launcher starts it in a
fresh container (see [how a run executes](../run-tests/how-a-run-executes.md)); the worker fetches its
specification, checks it, runs the plan and reports every event until the
backend acknowledges it. This page covers the worker's entry point, its
checks, exit codes and environment details. For the image, see [the worker image](../operate/worker-image.md). Launching and supervising containers is the backend's
job.

## Entry point

```text
saci-worker                     # started by the backend
saci-worker --spec run.json     # run a specification file locally
```

Started by the backend, the worker reads:

| Variable | Meaning |
| --- | --- |
| `SACI_BACKEND_URL` | Backend base URL as the worker reaches it (`backend.worker_url`). |
| `SACI_RUN_ID` | The run this worker was assigned. |
| `SACI_WORKER_TOKEN` | Run-scoped token for the worker routes; never reaches the tools. |
| `SACI_WORKSPACE` | Working directory of the run's shell; `/workspace` in the image, else the current directory. Also `--workspace`. |
| `NAME` for `llm.api_key: env:NAME`, otherwise `SACI_LLM_API_KEY` | The model API key, if the provider needs one. |
| `SACI_SECRET_<NAME>` | The value of each target secret `NAME`. |

See [security](../operate/security.md) for where each credential goes.

`--spec <file>` runs a [run specification](run-contracts.md) from a file
instead and writes every event to standard output (or `--events <file>`) as
one JSON object per line. Nothing is sent to a backend. It is how the image
is checked, and a way to rerun a stored specification by hand.

## What is checked before anything runs

The worker rejects its input, and runs nothing, when:

- `SACI_BACKEND_URL`, `SACI_RUN_ID` or `SACI_WORKER_TOKEN` is missing (exit 2);
- the specification has an unsupported `schema_version` (the backend and
  the image come from different releases);
- the specification names a run other than `SACI_RUN_ID`;
- the plan bundle does not validate, or an investigation names no repository;
- a configured secret or the LLM key is not in the worker's environment;
- the workspace is not a writable directory.

A rejected specification is reported as a single `run_finished` event with
outcome `error` and the reason, so the run ends with an explanation rather
than a lost worker. No model request is made and no tool runs.

## Investigation mode

A specification with an `investigation` field starts an investigation of a
failed unit (see the [design](../contributing/design/investigations.md)). The worker reports
it as a run with one unit, `investigation`, and one attempt. The steps are:

1. For each repository, the worker clones `url` into
   `<workspace>/src/<name>` and checks out `ref` (a commit, a tag, a branch
   or a remote branch) as a detached commit. The shell records this as
   evidence. A failed clone or checkout ends the run `error` before the
   first model request.
2. The worker runs each `prepare` command in each clone and records the
   output as evidence. The commands get the failure values as environment
   variables (see [configuration](configuration.md#investigations)). A
   failed command does not stop the investigation.
3. The model gets the failed unit, its attempts, the evidence of the
   failed run, the clones with their commits, and the preparation output.
   It can use the shell, the MCP servers of the environment, and the
   `conclude` tool.
4. The model calls `conclude` once with its result. The worker refuses a
   result that cites evidence that this investigation did not record, or a
   code location that is not in the clone (an unknown path, or a line
   after the end of the file). The model gets the reason and can try again.

The outcome of the attempt and the run follows the result: `concluded`
gives `passed`, and `inconclusive` gives `inconclusive`. An investigation
that ends without a result ends `error`, or `cancelled` when it is
cancelled. The result is the `investigation_concluded` event and the
`investigation` field of the report.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | The run ended and every event was delivered, whatever its outcome. |
| 1 | The worker could not set up its runtime. |
| 2 | The input was rejected; nothing was executed. |
| 3 | The specification could not be fetched or the events could not be delivered (the backend stayed unreachable for five minutes or refused them). |

The run's outcome is in its report, not the exit code. Stopping the
container (SIGTERM) or Ctrl-C cancels the run: the current tool gets a short
grace period, the remaining units are skipped, and the end is reported
before the worker exits.

## Process 1

The container runtime starts `saci-worker` as PID 1. The kernel gives each
orphaned process to PID 1, and only PID 1 can reap it. Processes become
orphans when the worker closes the run's shell, or when a plan starts a
background process that outlives its parent. If nothing reaps them, they
stay as `<defunct>` (zombie) processes.

When `saci-worker` is PID 1, it does the work of an init process:

1. It starts `saci-worker` again, with the same arguments, as its child.
   The child runs the run.
2. It sends SIGTERM, SIGINT, SIGHUP and SIGQUIT to the child.
3. It reaps each process that ends, including the orphans.
4. It exits with the exit code of the child. If a signal stops the child,
   the exit code is 128 plus the signal number.

Thus `ps` in a worker container shows two `saci-worker` processes. You do
not need `--init` or a separate init such as `tini`. When the worker is not
PID 1 (for example, `saci-worker --spec` on a host), it runs directly.

## Environment details

When the run starts, the worker reports where it executes in the
`run_started` event. The backend keeps this in the run report as
`worker_environment`. The web console shows it in the *Environment* view of
the run page, together with the worker image and the backend build.

| Field | Value |
|-------|-------|
| `saci` | The worker's Saci `version` and the `commit` it was built from ([build commit](../contributing/development.md#build-commit)). |
| `os` | `PRETTY_NAME` from `/etc/os-release`, or the `sw_vers` product name and version on macOS. |
| `kernel` | The output of `uname -sr`. |
| `arch` | The CPU architecture, for example `x86_64` or `aarch64`. |
| `tools` | The `name` and `version` of each known tool on `PATH`. |

The known tools are `bash`, `curl`, `git`, `jq`, `ripwire`, `ssh`, `nc`, `dig`,
`kubectl`, `oc`, `helm`, `podman`, `docker`, `java`, `mvn`, `jbang`, `node`
and `python3`. The list is `TOOLS` in `crates/saci-worker/src/environment.rs`.
The worker runs the version command of each tool at the same time, with
only `PATH` and `HOME` from its environment. It records the first line of
the output. It does not record a tool that is not installed, that fails, or
that does not reply in 5 seconds.

The backend records its own build in the report as `backend_build` when it
accepts the run. If the backend and the worker were built from different
commits, the console shows a warning.

## Next steps

- [The worker image](../operate/worker-image.md): build, pull and extend the image.
- [Run contracts](run-contracts.md): the specification and the events.
