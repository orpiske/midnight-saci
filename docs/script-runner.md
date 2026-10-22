# MCP script runner

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

Install the binary with `make install` ([command line](cli.md#installing)),
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
backend configuration ([MCP servers](mcp.md)):

```yaml
mcp_servers:
  scripts:
    url: http://script-runner:3000/mcp
    headers:
      Authorization: Bearer ${SCRIPT_RUNNER_TOKEN}
```

## Configuration

```yaml
server:
  bind: "127.0.0.1:3000"
  endpoint: /mcp

execution:
  max_concurrent: 4
  default_timeout_seconds: 30
  max_output_bytes: 65536
  working_directory: /opt/saci/script-runner
  environment:
    PATH: /usr/local/bin:/usr/bin:/bin

tools:
  - name: health_check
    description: Check application health
    executable: /opt/saci/scripts/health.sh
    read_only: true

  - name: deploy_application
    description: Deploy an approved application version
    executable: /opt/saci/scripts/deploy.sh
    timeout_seconds: 120
    arguments:
      - name: environment
        type: string
        required: true
        enum: [staging, production]
        binding: {kind: option, flag: --environment}
      - name: version
        type: string
        required: true
        pattern: '^[a-zA-Z0-9._-]+$'
        binding: {kind: option, flag: --version}
```

A call to `deploy_application` with
`{"environment": "staging", "version": "1.4.2"}` runs:

```text
/opt/saci/scripts/deploy.sh --environment staging --version 1.4.2
```

All sections reject unknown fields.

### `server`

| Setting | Default | Meaning |
|---------|---------|---------|
| `bind` | `127.0.0.1:3000` | Listen address. An address that is not loopback requires `auth.token`. |
| `endpoint` | `/mcp` | Path of the MCP endpoint. It cannot be `/health`. |
| `auth.token` | none | The bearer token, as `{ env: VAR }`: the name of an environment variable of the runner, the same format as Saci [secrets](configuration.md). If set, each MCP request must send `Authorization: Bearer <token>`. If the variable is not set or is empty, the runner does not start. |
| `allowed_hosts` | `localhost`, `127.0.0.1`, `::1` | Permitted `Host` header values. This prevents DNS rebinding. Add the host names that clients use when they connect from other hosts. |
| `allowed_origins` | none | Permitted `Origin` header values, for example `https://app.example.com`. The runner rejects a request that sends an `Origin` that is not in this list with `403`. Requests without `Origin` are permitted. |

### `execution`

| Setting | Default | Limits | Meaning |
|---------|---------|--------|---------|
| `max_concurrent` | 4 | 1 to 256 | Scripts that can run at the same time. |
| `default_timeout_seconds` | 30 | 1 to 3600 | Deadline for a tool that has no `timeout_seconds`. |
| `max_output_bytes` | 65536 | 1 to 16 MiB | Bytes kept from standard output and, separately, from standard error. |
| `working_directory` | required | absolute directory | Working directory of all scripts. |
| `environment` | empty | | The complete environment of all scripts. The runner does not pass its own environment to scripts. Set `PATH` if the scripts need it. |

### `tools`

| Setting | Meaning |
|---------|---------|
| `name` | Tool name: 1 to 64 letters, digits, `_` or `-`. Names must be unique. |
| `description` | Description for the client. Must not be empty. |
| `executable` | Absolute path of the script. See [Script checks](#script-checks). |
| `timeout_seconds` | Deadline for this tool, 1 to 3600. |
| `read_only` | Default `false`. Set `true` only if the script changes nothing. |
| `arguments` | Parameters, in the order that their values go on the command line. |

### `arguments`

| Setting | Meaning |
|---------|---------|
| `name` | Parameter name: a letter or `_`, then letters, digits or `_`, at most 64. Names must be unique in the tool. |
| `description` | Description for the client. Optional. |
| `type` | `string`. No other type is supported. |
| `required` | Default `false`. If an optional parameter is not in the call, nothing goes on the command line for it. |
| `enum` | Permitted values. The list must not be empty or contain duplicates. |
| `pattern` | A regular expression that the value must match. The runner uses the Rust `regex` syntax. The pattern is not anchored: add `^` and `$` to match the full value. |
| `binding.kind` | `option` or `positional`. |
| `binding.flag` | For `option`: the flag, `-` or `--` followed by letters, digits, `_` or `-`. A flag can be bound only once in a tool. |

An `option` binding adds the flag and then the value, as two arguments. A
`positional` binding adds the value. The order is the configuration order,
not the order of the keys in the call.

### Value rules

The runner rejects a call when:

- The tool name is not configured (JSON-RPC error `-32602`).
- An argument is not configured, or a required argument is missing.
- A value is not a string, is not in `enum`, or does not match `pattern`.
- A value starts with `-`. Thus a value cannot act as a flag of the script.
  For the same reason, `enum` values cannot start with `-`.
- A value is longer than 4096 bytes or contains a NUL character.

### Script checks

At startup, each `executable` must:

- Be an absolute path.
- Be a regular file. A symbolic link is rejected: configure the target path.
- Have an execute permission bit.
- Not be writable by its group or by other users.

A script that you start from a checkout can have group write permission if
your `umask` is `002`. Remove it with `chmod go-w`.

## Results

A completed call returns this structured content:

```json
{
  "execution_id": "40d656a4-0e48-4b3a-9e60-ff7b3cd30c03",
  "status": "completed",
  "exit_code": 0,
  "signal": null,
  "duration_ms": 12,
  "stdout": "Application is healthy\n",
  "stderr": "",
  "stdout_truncated": false,
  "stderr_truncated": false
}
```

| `status` | `isError` | Meaning |
|----------|-----------|---------|
| `completed` | false | The script exited with code 0. |
| `failed` | true | The script exited with another code, or a signal stopped it (`signal`). |
| `timed_out` | true | The deadline expired. The runner stopped the script. |
| `cancelled` | true | The client cancelled the call (`notifications/cancelled`), or the runner stopped. |
| `spawn_failed` | true | The script did not start. `error` gives the cause. |
| `rejected` | true | The arguments are not valid, or `max_concurrent` scripts are running. Nothing ran. `error` gives the cause. |

`exit_code` is `null` when the script did not exit normally. Output that is
not valid UTF-8 is converted with replacement characters (`U+FFFD`). When an
output stream is longer than `max_output_bytes`, the runner keeps the first
bytes and sets `stdout_truncated` or `stderr_truncated`.

Treat the output as data from the script, not as instructions.

## Execution

- Each script starts in a new process group, with `/dev/null` as standard
  input, the configured working directory and only the configured
  environment.
- The runner reads standard output and standard error at the same time. It
  keeps at most `max_output_bytes` of each and discards the rest, so the
  script does not block on a full pipe.
- When the script exits, the runner sends `SIGKILL` to the processes that
  remain in its process group. A script cannot leave background processes.
- At the deadline, or on cancellation, the runner sends `SIGTERM` to the
  process group. After 2 seconds it sends `SIGKILL`.
- After the script ends, the runner waits at most 1 second for the output
  pipes to close.
- If `max_concurrent` scripts are running, a new call is rejected at once.
  Calls do not wait in a queue.
- The runner does not retry a call.

Limitations: a process that starts a new session (`setsid`) leaves the
process group, and the runner cannot stop it. Use a container or a cgroup to
contain such processes. The process group handling works on Linux and
macOS. Windows is not supported.

## Audit events

The runner writes one log event for each call, with the target
`saci_script_runner::audit`. The event contains the execution ID, the tool
name, the client name from the MCP initialization, the status, the exit
code, the signal, the duration and the truncation flags. A rejected call
logs the reason. The runner does not log argument values or script output.

## Health endpoint

`GET /health` returns `{"status": "ok"}` when the process is up. It does not
require the token and gives no other data.

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

The Saci image ([the worker](worker.md#the-image)) contains
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

## Development

The code is in `crates/saci-script-runner`:

| Module | Owns |
|--------|------|
| `config` | YAML types and startup checks. |
| `registry` | The fixed tool list, input schemas and argument validation. |
| `executor` | Process execution, deadlines, output limits and process group cleanup. It does not use MCP. |
| `server` | The `rmcp` handler, the HTTP router, the token and `Origin` checks, and audit events. |

The runner uses `rmcp` 2.2 (the workspace version) in stateful Streamable
HTTP mode. Stateful mode is necessary because a cancellation notification
must reach the session that runs the call. The tests in
`crates/saci-script-runner/tests` start the server on a local port and use
the `rmcp` client over HTTP.
