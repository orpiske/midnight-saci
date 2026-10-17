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

Build the worker image and run a fixture-backed plan in it with
`make image-check` ([the worker](worker.md#the-image)); CI does the same in
its `worker-image` job.

`Cargo.lock` is committed so a fresh checkout resolves the same dependency
versions. Use `cargo build --locked` to verify that.

## Workspace layout

| Crate | Kind | Owns |
|-------|------|------|
| `saci-core` | library | Contracts shared by every process: plan model, parser and bundles, YAML configuration, run/step/attempt/evidence/event/report types, outcome aggregation, redaction. No network, process or database access. |
| `saci-mcp` | library | MCP client connections (stdio and Streamable HTTP through `rmcp`), tool selection and argument checks against tool schemas. Used by the worker for step tools and by the backend for outcome actions. |
| `saci-worker` | library + `saci-worker` binary | Per-run execution: the agent-library integration, shell, HTTP and MCP tools, sequential plan execution, and reporting events to the backend. |
| `saci-backend` | library + `saci-backend` binary | The REST API, SQLite run records, worker launchers (Podman/Docker and Kubernetes Jobs), supervision and outcome actions. |
| `saci-testkit` | library (test only) | Controlled fixtures: a scripted OpenAI-compatible model server, a fake MCP server, a fake orders service and a fake Kubernetes API. |
| `saci-cli` | `saci` binary | Offline authoring (`saci new test-plan`), backend startup, and run submission, progress, report and cancellation commands. |

Dependencies only point "down": `saci-cli` → `saci-backend`/`saci-core`,
`saci-backend` → `saci-mcp`/`saci-core`, `saci-worker` → `saci-mcp`/`saci-core`,
`saci-mcp` → `saci-core`. The worker and the
backend never link each other; they communicate through the REST contract in
`saci-core`.

## Quality gates

Run every gate CI runs with:

```sh
make check   # cargo fmt --check, cargo clippy -D warnings, cargo test
```

CI (`.github/workflows/ci.yml`) runs the same three jobs on every push to
`main` and on pull requests, using the pinned toolchain.

The lint baseline is adopted from the
[Wanaku Rust project](https://github.com/wanaku-ai/wanaku):

- `clippy.toml` holds Wanaku's thresholds (for example 30-line functions,
  five arguments, cognitive complexity 25), its MSRV and its disallowed
  methods (`std::thread::sleep`, `std::thread::spawn` and interactive stdin
  reads in favor of Tokio equivalents).
- `[workspace.lints]` in the root `Cargo.toml` enables Wanaku's Rust and
  Clippy lints: no `unsafe`, no `unwrap`/`expect`/`panic` outside tests,
  and the correctness, clarity, complexity and async groups. Every crate
  inherits them with `[lints] workspace = true`; new crates must do the same.
- CI treats every warning as an error (`-D warnings`).

Scoped exceptions:

- Unit tests may use `unwrap`, `expect` and `panic` through
  `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]`
  at each crate root, and integration test files allow the same three lints.
  Production code never does.
- Any other `#[allow(...)]` must sit on the narrowest item and carry a
  comment that explains why.
