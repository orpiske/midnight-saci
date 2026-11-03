# Make scripts available as MCP tools

`saci-script-runner` is a Saci module. It makes administrator-configured
scripts available as MCP tools over Streamable HTTP. Each script is one tool.
An MCP client, for example a Saci worker, Wanaku or an agent, can list the
tools and call them.

The runner is part of the Saci distribution: `make install` installs it and
the Saci image contains it. It is not necessary to run Saci. It is a separate
binary, no other Saci binary depends on it, and Saci does not start it. To
use it, write a configuration and start it yourself.

## How it works

1. At startup, the runner reads a YAML configuration and checks all of it.
   If one check fails, the runner does not start.
2. `tools/list` shows one tool for each configured script, with its
   description and a JSON Schema of its arguments.
3. On `tools/call`, the runner examines the arguments again against the
   configuration. Then it runs the configured executable with the values as
   separate arguments. It does not use a shell to make a command line.
4. The runner returns the exit code and the output as structured content,
   and as JSON text for clients that do not read structured content.

The client cannot select an executable, a flag, a working directory or an
environment variable. It can only select a configured tool and give values
that the configuration permits.

## Run it

Install the binary with `make install` ([install Saci](../get-started/install.md)),
or use the Saci image (see [Container](#container)).

The example in `examples/script-runner/` uses absolute script paths below
`/opt/saci-script-runner`. To try it from a checkout, change the paths and
start the runner:

```sh
sed "s|/opt/saci-script-runner|$PWD/examples/script-runner|" \
    examples/script-runner/config.yaml > /tmp/script-runner.yaml
saci-script-runner --config /tmp/script-runner.yaml --check-config
saci-script-runner --config /tmp/script-runner.yaml
```

| Option | Meaning |
|--------|---------|
| `--config`, `-c` | Configuration file. Default `script-runner.yaml`. |
| `--check-config` | Check the configuration, including the scripts, and stop. Exit code 0 means the configuration is valid. |

Set the log level with `RUST_LOG`, for example `RUST_LOG=info`. `SIGINT` and
`SIGTERM` stop the server. Before it stops, the runner stops all running
scripts and waits until they are reaped.

### Example session

Connect an MCP client to `http://127.0.0.1:3000/mcp`. With `curl`:

```sh
# Send one JSON-RPC message. The commands work in bash and zsh.
mcp() {
  curl -s -D headers.txt "$@" \
    -H 'content-type: application/json' \
    -H 'accept: application/json, text/event-stream' \
    http://127.0.0.1:3000/mcp -d "$BODY"
}
BODY='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"1"}}}' mcp
SID=$(grep -i mcp-session-id headers.txt | cut -d' ' -f2 | tr -d '\r')
session() { mcp -H "mcp-session-id: $SID" -H 'mcp-protocol-version: 2025-06-18'; }
BODY='{"jsonrpc":"2.0","method":"notifications/initialized"}' session
BODY='{"jsonrpc":"2.0","id":2,"method":"tools/list"}' session
BODY='{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"greet","arguments":{"name":"Ada","language":"pt"}}}' session
```

The last call runs `greet.sh --name Ada --language pt` and returns
`"stdout":"Olá, Ada!\n"`.

If `server.auth.token` is set, add `-H "authorization: Bearer $TOKEN"` to
the `curl` command in `mcp`.

To use the runner from Saci test plans, add it as an HTTP server in the
backend configuration ([MCP servers](mcp-servers.md)):

```yaml
mcp_servers:
  scripts:
    url: http://script-runner:3000/mcp
    headers:
      Authorization: Bearer ${SCRIPT_RUNNER_TOKEN}
```

To send the results of each run by email, see
[email the results of each run](email-run-results.md).

## Configure the runner

The configuration file lists the tools, their arguments and the execution
limits. A tool runs one absolute script path, and each argument has a
binding to a flag or a position. See the
[script runner configuration](../reference/script-runner-configuration.md)
for every setting, the value rules, the call results and the execution
rules.

## Security

Trust model:

- MCP clients are not trusted. They can call any configured tool with any
  values that the configuration permits.
- Administrators who can change the configuration, the scripts, or the
  directories that contain them are trusted. Whoever can write a script, or
  replace a directory in its path, can run commands as the runner user.
- The runner is not a sandbox. A script can do anything that the runner
  user can do.

Do these steps:

1. Write the scripts so that they are safe for all values that the
   configuration permits. The runner passes values as separate arguments,
   but a script that uses a value in `eval` or in an unquoted expansion can
   still be attacked. Use `enum` or a strict `pattern` for each value.
2. Make sure that only administrators can write the scripts and all of the
   directories in their paths. The startup check examines only the script
   file. A change after startup takes effect at the next call.
3. Run the runner as a dedicated user that is not root. In a container, use
   a read-only root file system and limit CPU, memory, the number of
   processes and network access.
4. Keep the default loopback bind, or set `auth.token`. For remote
   access, also put the runner behind a gateway that authenticates clients,
   and make sure that clients cannot connect to the runner directly. Set
   `allowed_hosts` to the names that clients use. CORS is not
   authentication.
5. Scripts can have side effects that you cannot reverse. If a client
   retries after it loses a response, the script runs again. Make such
   scripts idempotent.

## Container

The Saci image ([the worker](../operate/worker-image.md)) contains
`/usr/local/bin/saci-script-runner`. To run the runner from the image, set it
as the entry point and mount your configuration and scripts read-only. The
image runs as UID 10001, which cannot write the mounted scripts.

A runner in a container must listen on all interfaces, so it needs a token:

```yaml
server:
  bind: "0.0.0.0:3000"
  auth:
    token: { env: SCRIPT_RUNNER_TOKEN }
  allowed_hosts: [script-runner, localhost, 127.0.0.1]
```

```sh
podman run --rm -p 127.0.0.1:3000:3000 --read-only \
    --pids-limit 256 --memory 256m \
    -e SCRIPT_RUNNER_TOKEN \
    -v ./runner.yaml:/etc/saci-script-runner/config.yaml:ro,Z \
    -v ./scripts:/opt/saci-script-runner/scripts:ro,Z \
    --entrypoint /usr/local/bin/saci-script-runner \
    localhost/saci-worker:latest --config /etc/saci-script-runner/config.yaml
```

The image does not contain the example scripts. The Helm chart does not
start the runner.

## Next steps

- [Script runner configuration](../reference/script-runner-configuration.md).
- [Email the results of each run](email-run-results.md).
- [Outcome actions](outcome-actions.md): call runner tools after a run.
