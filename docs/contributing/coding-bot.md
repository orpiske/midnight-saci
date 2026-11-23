# Build and run the coding bot

The coding bot is an Amaru Light instance that changes, builds and checks
this repository. This page tells you how to build its container image, start
it and make it ready for a job.

## Prerequisites

- Podman.
- A checkout of Amaru Light, to build the base image.
- A GitHub token for `orpiske/midnight-saci`. Use a fine-grained token with
  these repository permissions:
  - Contents: read and write.
  - Pull requests: read and write.
- An account for Claude Code or for Codex.

## The image

The image definition and its configuration are in the
[`coding-bot/`](../../coding-bot/README.md) folder. The image extends the
Amaru Light base image `localhost/mini-amaru:latest` and adds:

- Claude Code and Codex, the agent CLIs.
- Rust, the version in `rust-toolchain.toml`, with `clippy` and `rustfmt`.
- Node.js 22 and npm, for the [web console](web-console.md).
- The C and C++ tools that the native crates compile with.
- `procps-ng`, because some worker tests use `ps`.
- `bubblewrap`, the sandbox that Codex uses.

The image keeps the entry point, the `/app` working directory and the
`mini-amaru` user of the base image.

| File | Contents |
|------|----------|
| `Containerfile` | The image definition. |
| `providers.yaml` | The agent CLIs and their command lines. |
| `models.yaml` | The logical model IDs. |
| `routing.yaml` | The model for each task. Claude is first and Codex is the fallback. |
| `project.yaml` | The repository, the checkout path and the check command. |

The check command is `make check`. The bot runs it in the job worktree.

> [!NOTE]
> The image cannot run the browser tests (`npm run e2e`) or
> `make image-check`. It has no browser and no container runtime.

## Build the image

1. In the Amaru Light checkout, build the base image:

   ```sh
   make image
   ```

2. In the root of this repository, build the project image:

   ```sh
   podman build -f coding-bot/Containerfile -t localhost/amaru-light-midnight-saci:dev coding-bot
   ```

The build context is the `coding-bot/` folder. Do not use the repository
root as the context.

The Rust version is set with the `SACI_RUST_TOOLCHAIN` build argument. Its
default value is the channel in `rust-toolchain.toml`. When you change the
toolchain in `rust-toolchain.toml`, change the default in the
`Containerfile` also.

## Start the container

The container must have a GitHub token in the `GH_TOKEN` environment
variable. The bot uses the token to clone the repository, push branches and
open pull requests. `gh` reads `GH_TOKEN`, and the image configures Git to
get its HTTPS credentials from `gh`. You do not need to run `gh auth login`
or `gh auth setup-git`.

1. Export the token in your shell:

   ```sh
   export GH_TOKEN=<your-token>
   ```

2. Start the container:

   ```sh
   podman run -d --name amaru-light-midnight-saci \
     -p 127.0.0.1:8765:8765 \
     -e GH_TOKEN \
     -v amaru-light-midnight-saci:/home/mini-amaru:U \
     localhost/amaru-light-midnight-saci:dev \
     serve -c projects/project.yaml --host 0.0.0.0 \
     --cors-origin http://127.0.0.1:8765 --cors-origin http://localhost:8765
   ```

`-e GH_TOKEN` without a value copies the variable from your shell. Thus the
token does not show in the command line or in the shell history.

The container keeps the token only in its environment. The volume does not
contain the token. When you create the container again, give the token
again.

The dashboard is at `http://127.0.0.1:8765`. If you change the host port,
change the `--cors-origin` values also.

### Storage

All state is in one named volume at `/home/mini-amaru`:

- `data/`: the job database, the logs and the worktrees. `/app/data` links
  to it.
- `repos/`: the checkouts. `/app/repos` links to it.
- The agent logins, in `~/.claude` and `~/.codex`.
- The Git configuration, in `~/.gitconfig`.

The Rust toolchain and the cargo cache are in `/opt`, not in the volume.
Thus a new image always supplies its toolchain.

> [!IMPORTANT]
> Use a named volume, not a host directory. Podman fills an empty named
> volume from the image, and this creates the `data` and `repos` folders. A
> host directory stays empty.

## Log in to the agent

Log in one time, after the first start. The credentials stay in the volume.
Log in to the agent that `routing.yaml` uses.

- Claude Code. Run this command, type `/login` and do the instructions:

  ```sh
  podman exec -it amaru-light-midnight-saci claude
  ```

- Codex:

  ```sh
  podman exec -it amaru-light-midnight-saci codex login --device-auth
  ```

The routes use Claude. To use only Codex, change the `claude-*` IDs in
`coding-bot/routing.yaml` to `codex-default` or `codex-classic`, and build
the image again.

If the bot makes commits, set the Git author:

```sh
podman exec amaru-light-midnight-saci git config --global user.name "Your Name"
podman exec amaru-light-midnight-saci git config --global user.email you@example.com
```

## Verify the container

1. Make sure that the service is available:

   ```sh
   curl -fsS http://127.0.0.1:8765/api/health
   ```

   The result is `{"ok":true,"project":"midnight-saci"}`.

2. Make sure that the agent and GitHub are ready:

   ```sh
   podman exec amaru-light-midnight-saci /app/mini-amaru doctor -c projects/project.yaml
   ```

   The result must show `[OK]` for the agent CLIs, the planning model and
   `gh authenticated`. On a new volume, the project directory does not
   exist. The bot clones the repository when a job needs it.

3. Run a small job. Examine the agent output and the `make check` result.
   A job can have a success status when the agent report says that the
   task is blocked.

## Next steps

- [Development](development.md): the quality gates that the bot runs.
- [Testing](testing.md): the test gates and the fixtures.
- [Build and develop the web console](web-console.md).
