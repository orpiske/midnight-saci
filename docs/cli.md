# The `saci` command line

`saci` is the client. It authors plans offline, starts a backend, and
submits plans to a local or remote backend through the [REST API](api.md).
It never starts workers or containers itself: the backend does.

## Connecting to a backend

Every command that talks to a backend accepts:

| Option | Default | Meaning |
| --- | --- | --- |
| `--backend <url>` | `SACI_BACKEND_URL`, else `http://127.0.0.1:8080` | Backend base URL. |
| `--token-env <name>` | `SACI_TOKEN` | Environment variable holding the client token. |

When the backend sets `backend.auth.token_env` (required when it listens
beyond loopback, see [configuration](configuration.md)), export the same
token in `SACI_TOKEN` on the client. The token is sent as a bearer token and
never printed.

## `saci backend start`

```sh
export OPENROUTER_API_KEY=...        # whatever llm.api_key_env names
saci backend start --config saci.yaml
```

Reads and validates the configuration, opens the database in
`backend.data_dir`, and serves the API on `backend.listen` in the foreground
until Ctrl-C or SIGTERM. Logs go to standard error; `RUST_LOG` adjusts them.
The `saci-backend` binary does the same and exists for container images.

An invalid configuration, a missing client token variable or an unusable
listen address or data directory exits 2 with the offending setting.

## `saci run`

```sh
saci run plan.md --environment staging [--policy keep-going] \
    [--idempotency-key <key>] [--json]
```

1. Reads `plan.md` and every procedure it invokes into a bundle and validates
   it locally ([test plans](test-plans.md)). Invalid plans exit 2 before the
   backend is contacted.
2. Submits the bundle with an `Idempotency-Key`, a fresh UUID unless
   `--idempotency-key` is given.
3. Prints the accepted run ID on standard output, and a one-line summary on
   standard error; `--json` prints the run summary instead.

`saci run` returns once the backend accepts the run; it does not wait for the
run to finish.

### When the reply is lost

If the connection breaks after the request was sent, the CLI cannot know
whether the run was accepted. It exits 4 and prints the key it used:

```text
error: the reply from http://127.0.0.1:8080 was lost (...); the request may or may not have taken effect.
Repeat the same command with --idempotency-key 0f8c...: the backend returns the run it already accepted instead of starting a second one.
```

Repeating the command with that key is always safe: the backend answers with
the run it already accepted (the summary says "already accepted for this
idempotency key"), or accepts it now. Reusing a key with a different plan,
environment or policy is refused with `idempotency_conflict`.

## Exit codes

| Code | Meaning | What to do |
| --- | --- | --- |
| 0 | Success. | |
| 2 | Invalid input: plan, configuration, arguments, an unknown environment or policy, or a run that does not exist. | Fix the input named in the message. |
| 3 | The backend is unreachable, refused the credentials, or failed. | Start the backend, check `--backend`, or set `SACI_TOKEN`; the message says which. |
| 4 | The outcome of a request is unknown. | Repeat it as the message says. |
