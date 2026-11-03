# Script runner configuration

This page is the reference for the `saci-script-runner` configuration file,
the call results and the execution rules. For how to run the runner, see
[MCP script runner](../connect/script-runner.md).

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
| `max_bytes` | The longest permitted value, in bytes. Default 4096. The maximum is 131072, the Linux limit for one command line argument. The schema gives it as `maxLength`. |
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
- A value is longer than `max_bytes` or contains a NUL character.

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

## Next steps

- [MCP script runner](../connect/script-runner.md): run it, connect it and
  secure it.
- [Email the results of each run](../connect/email-run-results.md): a
  complete example.
