# The worker and its image

A worker executes exactly one run. The backend's launcher starts it in a
fresh container (see [execution](execution.md)); the worker fetches its
specification, checks it, runs the plan and reports every event until the
backend acknowledges it. This page covers the worker's entry point and the
`saci-worker` image. Launching and supervising containers is the backend's
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
| the variable named by `llm.api_key_env` | The model API key, if the provider needs one. |
| `SACI_SECRET_<NAME>` | The value of each target secret `NAME`. |

See [security](security.md) for where each credential goes.

`--spec <file>` runs a [run specification](contracts.md) from a file
instead and writes every event to standard output (or `--events <file>`) as
one JSON object per line. Nothing is sent to a backend. It is how the image
is checked, and a way to rerun a stored specification by hand.

## What is checked before anything runs

The worker rejects its input, and runs nothing, when:

- `SACI_BACKEND_URL`, `SACI_RUN_ID` or `SACI_WORKER_TOKEN` is missing (exit 2);
- the specification has an unsupported `schema_version` (the backend and
  the image come from different releases);
- the specification names a run other than `SACI_RUN_ID`;
- the plan bundle does not validate;
- a configured secret or the LLM key is not in the worker's environment;
- the workspace is not a writable directory.

A rejected specification is reported as a single `run_finished` event with
outcome `error` and the reason, so the run ends with an explanation rather
than a lost worker. No model request is made and no tool runs.

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

## The image

[`Containerfile`](../Containerfile) builds `saci-worker` in a Rust stage and
copies it into `debian:trixie-slim` with the utilities test plans commonly
use: `bash`, `curl`, `jq`, `git`, `ssh`, `ps`, `ip`, `dig` and `nc`, plus the
CA certificates the HTTP tool and the model connection need. The image
also contains `saci-backend` and `saci`, so the Helm chart runs the backend
from the same image with `saci-backend --config ...` ([Helm](helm.md)).
The entry point stays `saci-worker`.

```sh
make image                               # podman, or docker if podman is missing
podman build -t localhost/saci-worker:latest .
make image-check                         # runs a fixture-backed plan in the image
```

`localhost/saci-worker:latest` is the default `worker.image`
([configuration](configuration.md#worker)).

- **Non-root.** The image runs as UID 10001 in group 0. `/workspace` (also
  `HOME`) is owned by group 0 and group-writable, so the arbitrary UIDs that
  OpenShift's restricted policy assigns work too. Nothing needs root, a
  container runtime socket or added capabilities.
- **Isolation.** Every run gets a new container; files a plan creates live
  only in that container's `/workspace`.
- **Checked in CI.** The `worker-image` job builds the image and runs a
  fixture-backed plan in it as UID 123456, against a scripted model.

### Adding utilities

Plans can only use what the image contains. Derive an image and point
`worker.image` at it:

```dockerfile
FROM localhost/saci-worker:latest
USER root
RUN apt-get update \
    && apt-get install --yes --no-install-recommends postgresql-client \
    && rm -rf /var/lib/apt/lists/*
USER 10001:0
```

Keep the `USER 10001:0` line (or any non-root user in group 0) and the
entry point. A plan that needs a tool the image lacks fails its step with
the shell's "command not found" in the evidence.
