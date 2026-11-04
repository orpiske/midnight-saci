# Install Saci

This page tells you how to build the Saci binaries and the worker image
from a checkout, and how to install them.

## Prerequisites

- Linux or macOS, on x86_64 or arm64.
- [rustup](https://rustup.rs/). The first `cargo` command installs the
  pinned toolchain from `rust-toolchain.toml`.
- Podman or Docker, to build the worker image and to run workers.
- Optional: Node.js 22 or later with npm, to embed the
  [web console](../run-tests/web-console.md).

## Install the binaries

To build the binaries in release mode and install them, run this command
from the repository root:

```sh
make install                      # installs in /usr/local/bin
make install BINDIR=$HOME/bin     # installs in a directory that you select
make install PREFIX=/opt/saci     # installs in /opt/saci/bin
```

The target installs `saci`, `saci-backend`, `saci-worker`,
`saci-script-runner` ([MCP script runner](../connect/script-runner.md)) and
`saci-julia` ([Julia 1 judge](../operate/julia-judge.md)). It uses only
`cargo` and the standard `install` tool, and works on Linux and macOS.

Do not run `sudo make install`. The target builds as the current user. If
you do not have write access to the directory, the target uses `sudo` only
to copy the binaries, and `sudo` asks for your password. To use a different
command, set `SUDO`, for example `make install SUDO=doas`. To disable it,
set `SUDO=`. Packagers can set `DESTDIR` to install into a staging root.

To embed the [web console](../run-tests/web-console.md), run `make web` before
`make install`. To remove the installed binaries, run `make uninstall` with
the same `PREFIX` or `BINDIR`.

## Get the worker image

Each run executes in a worker container. Build the image from the
checkout:

```sh
make image                        # localhost/saci-worker:latest
```

`localhost/saci-worker:latest` is the default `worker.image`. To use the
published image instead, see [the worker image](../operate/worker-image.md).

## Install the agent skills (optional)

Saci contains skills for coding agents such as Claude Code, Codex, Cursor,
GitHub Copilot, Gemini CLI and OpenCode. The skills teach the agent to
convert plans, run them, summarize the report and investigate failures.
To install all skills for Claude Code in the current project, run:

```sh
saci skill install --all --agent claude
```

For other agents and options, see
[use Saci from a coding agent](../run-tests/use-agent-skills.md).

## Next steps

- [Quickstart](quickstart.md): run plans end to end with a scripted model.
- [Run with a real model](run-with-a-real-model.md): connect Saci to your
  model provider and your system.
- [Use Saci from a coding agent](../run-tests/use-agent-skills.md): let
  your agent convert and run plans.
