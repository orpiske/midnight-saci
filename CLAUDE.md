# CLAUDE.md

Guidelines for working on Midnight Saci, an AI test engineer that runs
Markdown test plans with an LLM agent.

## Repository layout

- `crates/`: the Rust workspace (`saci-core`, `saci-mcp`, `saci-worker`,
  `saci-backend`, `saci-cli`, `saci-testkit`, `saci-script-runner`,
  `saci-julia`).
  Dependencies point down only.
  See [docs/development.md](docs/development.md#workspace-layout).
- `web/`: the web console (TypeScript, React, Vite). The backend serves it at
  `/ui/`.
- `docs/`: user and contributor documentation. `docs/openapi.yaml` is the REST
  contract. `docs/plans/` holds the system plan and the ticket breakdown.
- `docs/templates/`: the test plan starter and example. The plan format is in
  [docs/test-plans.md](docs/test-plans.md).
- `test/plans/`: Saci test plans for Saci itself.
- `artwork/BRAND.md`: the visual identity. Follow it for all UI work.
- `examples/`, `deploy/`: configuration and deployment examples.

## Build and check

```sh
make check       # cargo fmt --check, clippy -D warnings, cargo test
make web-check   # web console checks, browser tests, backend console tests
make console     # backend and saci with the console embedded (debug by default)
make image-check # build the worker image and run a plan in it
```

Run `make check` before you push. Run `make web-check` when you change `web/`
or the code that serves it. CI runs the same gates.

## Engineering rules

- Simplicity over complexity. Do not add an abstraction until a second real
  use needs it.
- Follow the lints in `Cargo.toml` and `clippy.toml`. Production code does not
  use `unwrap`, `expect`, `panic` or `unsafe`. Put each other `#[allow]` on the
  narrowest item, with a comment that gives the reason.
- Do not mix languages. Write tools, skills and automation in Rust or in
  standard bash. Do not use Python, Perl or similar. Bash scripts must work on
  Linux and macOS.
- Target platforms are Linux and macOS. Windows support is optional and low
  priority.
- Debug and release builds have the same features. For example, every build
  embeds the web console when `web/dist` exists.
- Keep `docs/openapi.yaml` and the generated web API types in sync with the
  backend.

## Tests

- Major features have tests.
- Tests must be meaningful: they check behavior that can break, not only that
  code runs.
- Use the fixtures in `saci-testkit` instead of real models or services.

## Documentation

- New features include documentation. Feature changes include updates to the
  related documentation.
- Write technical content in ASD-STE100 Simplified Technical English. This
  includes procedures, command explanations, configuration, rules, endpoints,
  system behavior and other accuracy-critical operational text.
- Be concise and professional. Avoid em-dashes.

## Commits and pull requests

- Commit ticket by ticket so each change traces to its issue. Put the issue
  number in the subject, for example `Serve the web console (#53)`, and add
  `Refs #N` in the body.
- Write the subject in the imperative mood. Use the body to explain why.
- Before you open a pull request, squash or amend similar commits (for
  example, fixups to the same ticket) into one.
- Keep pull request descriptions short and factual. Avoid em-dashes.
