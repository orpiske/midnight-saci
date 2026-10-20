# Launching workers with Podman or Docker

The backend starts one container per accepted run, from `worker.image`
([the worker image](worker.md#the-image)), with the `podman` or `docker`
command-line program. It needs no socket, no API client library and no
other service: only the program on the backend's `PATH` and a working
runtime behind it.

## Choosing the runtime

| `worker.runtime` | Uses |
| --- | --- |
| `auto` (default) | `podman` if it is on the backend's `PATH`, otherwise `docker`. |
| `podman` | `podman` from the backend's `PATH`. |
| `docker` | `docker` from the backend's `PATH`. |
| `kubernetes` | Kubernetes Jobs instead; see [Kubernetes Jobs](kubernetes.md). |

The backend looks the program up in its own `PATH`, the way a shell would,
and runs `<runtime> version` when it starts. Nothing else is searched: no
installation directories, no sockets, no automatic installation. When the
program is missing or not executable, or `version` fails (for example
because the Docker daemon is not running), the backend logs the reason and
keeps serving; every run submitted meanwhile ends `error` with that reason,
for example:

```text
the worker could not be launched: no usable container runtime for worker.runtime `auto`:
podman is not in the backend's PATH; docker is not in the backend's PATH. Install Podman or
Docker, or put it on the backend's PATH (PATH=/usr/bin:/bin)
```

Fix it and restart the backend.

## What a worker is started with

For run `run-1a2b…` the backend runs, as an argument list and never through
a shell:

```text
podman run --detach --name=saci-run-1a2b… \
  --label=io.saci.managed=true --label=io.saci.run=run-1a2b… \
  --security-opt=no-new-privileges \
  [--network=<worker.network>] [--add-host=<entry>]… \
  [--cpus=<cpu>] [--memory=<memory>] \
  --env SACI_BACKEND_URL --env SACI_RUN_ID --env SACI_WORKER_TOKEN \
  [--env <llm.api_key variable>] [--env SACI_SECRET_<NAME>]… \
  <worker.image>
```

- **Stable identity.** The container is named after its run and labelled
  with it. A second start for the same run fails on the name instead of
  creating a duplicate, and the backend finds its workers again after a
  restart by the `io.saci.managed` label.
- **Credentials.** `--env NAME` without a value makes the runtime copy the
  value from its own environment, which the backend sets for that one
  command. Token, LLM key and secret values therefore never appear in an
  argument list, `ps` output or the runtime's command log. Only the
  variables listed are passed; the rest of the backend's environment stays
  out of the container.
- **No host directories.** Nothing is mounted. The worker fetches its
  specification from the backend with its token, and its `/workspace` lives
  and dies with the container.
- **Resources.** `environments.<env>.worker.cpu` and `.memory` become
  `--cpus` and `--memory`, in the runtime's own syntax (`1.5`, `2g`).

Cancelling a run, or a run overrunning its deadline, stops its worker with
`<runtime> stop --time=30`: SIGTERM, which makes the worker cancel and
report its end, then SIGKILL if it has not exited. Once the run has ended
the worker is removed with `<runtime> rm --force`. See
[supervision](supervision.md) for deadlines, lost workers and restarts.

## Worker image

The backend records the image of each worker in the run report, as
`worker_image`. The web console shows it on the run page. It has these
fields:

| Field | Value |
|-------|-------|
| `reference` | The configured `worker.image`, with its tag. |
| `digest` | The registry digest, `sha256:<hex>`. It is absent for an image that you built locally and did not push. |
| `id` | The local image ID, `sha256:<hex>`. |

The backend gets these values from the runtime after it starts the worker,
and before it removes the worker:

- **Podman and Docker.** `<runtime> inspect --format={{.Image}}` gives the
  image ID. `<runtime> image inspect --format={{json .RepoDigests}}` gives
  the registry digest of the configured repository, or else the first
  digest in the list.
- **Kubernetes.** The `imageID` of the worker container in the pod status
  gives the registry digest. The kubelet reports it when the container
  starts, so the report has only `reference` while the pod is pending.

If the runtime cannot give a value, the report does not include it. A tag
such as `latest` can point to a different image later, so use the digest
or the image ID to identify the exact image that ran.

## Networking

A worker needs three connections, all configured explicitly:

| From the worker to | Configured by |
| --- | --- |
| the backend | `backend.worker_url`, the backend's URL *as the container sees it* |
| the model provider | `llm.base_url` |
| the systems under test | the environment's `parameters`, and the plan |

`worker.network` is passed as `--network`, and each `worker.add_hosts`
entry as `--add-host`. Nothing is discovered automatically.

### Linux, host network (simplest)

The worker shares the host's network, so `127.0.0.1` means the host for
the worker too. The backend can stay on loopback without client
authentication, and a host-local Ollama works unchanged.

```yaml
backend:
  listen: 127.0.0.1:8080
  worker_url: http://127.0.0.1:8080
worker:
  network: host
llm:
  base_url: http://127.0.0.1:11434/v1   # Ollama on the host
  model: qwen3:8b
```

This is the setup the integration tests use, with both Docker and Podman
(rootless Podman supports `--network=host`). The worker can then reach
everything the host can, including other services on its loopback.

### Bridge network with a host alias

To keep workers off the host's loopback, use the runtime's default bridge
(or a named network) and reach the host through an alias. The backend must
then listen on an address the bridge can reach, which is not loopback, so
client authentication is required.

Podman (4.1 or later) adds `host.containers.internal` by itself:

```yaml
backend:
  listen: 0.0.0.0:8080
  worker_url: http://host.containers.internal:8080
  auth:
    token_env: SACI_TOKEN
llm:
  base_url: http://host.containers.internal:11434/v1
```

Docker Engine on Linux needs the alias added:

```yaml
backend:
  listen: 0.0.0.0:8080
  worker_url: http://host.docker.internal:8080
  auth:
    token_env: SACI_TOKEN
worker:
  add_hosts: ["host.docker.internal:host-gateway"]
llm:
  base_url: http://host.docker.internal:11434/v1
```

Ollama listens on `127.0.0.1:11434` by default, which a bridged container
cannot reach; start it with `OLLAMA_HOST=0.0.0.0:11434`. Firewalls such as
`firewalld` or `ufw` may also need to allow the bridge to reach ports 8080
and 11434.

### Targets in containers

To test services that run in containers, put them and the workers on one
named network and address them by container name:

```sh
podman network create saci-tests
podman run -d --name orders --network saci-tests orders:latest
```

```yaml
worker:
  network: saci-tests
environments:
  local:
    parameters:
      ORDERS_URL: http://orders:8080
```

`backend.worker_url` must then name an address reachable from that network
too, for example the host alias above with `add_hosts`.

### Docker Desktop (macOS, Windows)

`host.docker.internal` is built in; use the bridge setup without
`add_hosts`. Host networking is limited on Docker Desktop and is not
recommended there.

## Tested platforms

| Platform | Runtime | Checked by |
| --- | --- | --- |
| Linux x86_64 (Ubuntu) | Docker Engine 29 | the CI `worker-image` job and local runs: the backend launches the image, the worker runs a scripted plan and reports it, the container is removed |
| Linux x86_64 (Ubuntu) | rootless Podman 4.9 (GitHub runner) | the same test in the CI `worker-image` job |
| any | fake `podman`/`docker` programs | unit and integration tests for `PATH` resolution, arguments, credential passing, state, listing and removal |

macOS and Windows (Docker Desktop, Podman machine) use the same commands
but are not part of the automated checks.

Run the checks yourself with `make image-check` (`RUNTIME=docker` or
`RUNTIME=podman` to choose).
