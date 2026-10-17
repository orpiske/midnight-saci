# Development

## Prerequisites

- [rustup](https://rustup.rs/). The pinned toolchain in `rust-toolchain.toml`
  (Rust 1.96.0 with `clippy` and `rustfmt`) is installed automatically on the
  first `cargo` command.
- Optional: Podman or Docker to build the worker image and run local workers.

The workspace uses Rust edition 2024. The minimum supported Rust version is
declared once as `rust-version = "1.96"` in `Cargo.toml` and matches the pinned
toolchain and the Clippy `msrv` setting.

## Build and run

```sh
cargo build                 # all crates
cargo run -p saci-cli -- --help
cargo run -p saci-backend -- --help
cargo run -p saci-worker -- --help
```

`Cargo.lock` is committed so a fresh checkout resolves the same dependency
versions. Use `cargo build --locked` to verify that.

## Workspace layout

| Crate | Kind | Owns |
|-------|------|------|
| `saci-core` | library | Contracts shared by every process: plan model, parser and bundles, YAML configuration, run/step/attempt/evidence/event/report types, outcome aggregation, redaction. No network, process or database access. |
| `saci-worker` | library + `saci-worker` binary | Per-run execution: the agent-library integration, shell, HTTP and MCP tools, sequential plan execution, and reporting events to the backend. |
| `saci-backend` | library + `saci-backend` binary | The REST API, SQLite run records, worker launchers (Podman/Docker and Kubernetes Jobs), supervision and outcome actions. |
| `saci-cli` | `saci` binary | Offline authoring (`saci new test-plan`), backend startup, and run submission, progress, report and cancellation commands. |

Dependencies only point "down": `saci-cli` → `saci-backend`/`saci-core`,
`saci-backend` → `saci-core`, `saci-worker` → `saci-core`. The worker and the
backend never link each other; they communicate through the REST contract in
`saci-core`.
