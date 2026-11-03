# The worker image

Each run executes in a new container from the worker image. This page
describes what the image contains, where to get it, and how to add the
tools that your plans need.

## What the image contains

[`Containerfile`](../../Containerfile) builds `saci-worker` in a Rust stage and
copies it into `registry.fedoraproject.org/fedora:44` with the utilities test
plans commonly use: `bash`, `curl`, `jq`, `git`, `ssh`, `ps`, `ip`, `dig` and
`nc`, plus the CA certificates the HTTP tool and the model connection need. The image
also contains [ripwire](https://github.com/redhat-et/ripwire), the code map
tool that [failure investigations](../run-tests/investigate-failures.md) use. A separate stage
builds it from the tag in the `RIPWIRE_VERSION` build argument. On x86_64,
ripwire needs a CPU with x86-64-v3 instructions (AVX2), as its release
binaries do. The image
also contains `saci-backend`, `saci` and `saci-script-runner`. Thus the Helm
chart runs the backend from the same image with `saci-backend --config ...`
([Helm](helm.md)), and the [MCP script runner](../connect/script-runner.md#container)
can run from it too.
The entry point stays `saci-worker`.

## Build or pull the image

```sh
make image                               # podman, or docker if podman is missing
podman build -t localhost/saci-worker:latest .
make image-check                         # runs a fixture-backed plan in the image
```

`localhost/saci-worker:latest` is the default `worker.image`
([configuration](../reference/configuration.md#worker)).

CI publishes `quay.io/orpiske/saci-worker:latest` and a tag with the full
commit SHA after a successful push to `main`. Each tag is a multi-arch
manifest for Linux x86_64 (`amd64`) and AArch64 (`arm64`). Set `worker.image`
to this image to use it. Set `worker.image_pull_policy: Always` to pull the
latest image before each run. For Helm, set
`--set image.repository=quay.io/orpiske/saci-worker --set image.tag=latest`.


- **Non-root.** The image runs as UID 10001 in group 0. `/workspace` (also
  `HOME`) is owned by group 0 and group-writable, so the arbitrary UIDs that
  OpenShift's restricted policy assigns work too. Nothing needs root, a
  container runtime socket or added capabilities.
- **Isolation.** Every run gets a new container; files a plan creates live
  only in that container's `/workspace`.

## Adding utilities

Plans can only use what the image contains. Derive an image and point
`worker.image` at it:

```dockerfile
FROM localhost/saci-worker:latest
USER root
RUN dnf install --assumeyes --setopt=install_weak_deps=False postgresql \
    && dnf clean all
USER 10001:0
```

Keep the `USER 10001:0` line (or any non-root user in group 0) and the
entry point. A plan that needs a tool the image lacks fails its step with
the shell's "command not found" in the evidence.

## Next steps

- [Run workers with Podman or Docker](container-workers.md): how the
  backend starts the image.
- [Worker reference](../reference/worker.md): the entry point, exit codes
  and environment details.
