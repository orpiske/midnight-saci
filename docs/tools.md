# Worker tools

The worker gives the model a small set of tools. Every call is validated,
recorded as redacted evidence (see [credentials and redaction](security.md))
and bounded by `limits.operation_timeout_secs` and
`limits.output_limit_bytes`.

## Shell

One `bash` session serves every shell call of a run. Variables, exported
variables, helper functions, the working directory and files persist
between calls and between steps, so a setup step can prepare what later
tests use. The session starts in a run-private workspace directory, which is
also `HOME` unless the worker environment sets one.

### Supported languages

Author-supplied command blocks run in this session when their fence language
is `bash`, `sh` or `shell`. Commands run under `bash` (not a POSIX `sh`), so
`sh` blocks must be valid `bash`, which is true of nearly all of them. Blocks
in any other language, including `console` transcripts with `$ ` prompts,
are reported as unsupported instead of being guessed at; run another
interpreter from a `bash` block if you need one (`python3 - <<'EOF' … EOF`).

### How a command runs

Each command is written to a file and sourced by the session, exactly as
written, with standard input from `/dev/null`. Saci adds no shell flags: a
command that runs `set -e` changes the session for later commands, as it
would in a terminal.

The evidence records the command, standard output and standard error
(separately, each bounded with visible truncation), the command's exit
status (`$?` of its last command) and how it ended:

| `termination` | Meaning | Session afterwards |
|---------------|---------|--------------------|
| `exited` | The command returned. A nonzero `exit_code` is evidence, not an error. | Kept. |
| `timed_out` | `limits.operation_timeout_secs` expired. | Kept when killing the command's processes ended it; lost (`session_lost: true`) when the command ran inside the shell itself, such as a `while` loop of builtins. |
| `cancelled` | The run was cancelled. | As for `timed_out`. |
| `session_ended` | The command ended the shell, for example with `exit 3`; `exit_code` is the shell's status. | Lost. |
| `session_lost` | The shell died or closed its output. | Lost. |

On a deadline or cancellation the worker kills every process descended from
the shell. A process that detaches completely (double-forks and is
re-parented) escapes this; the worker container's removal at the end of the
run stops it.

When a session is lost, the next shell call starts a new one and its
evidence has `restarted: true`: variables, functions and the working
directory are gone, files in the workspace remain. Saci never presents a new
session as the old one.

A command that permanently redirects the shell's own output (`exec >file`)
hides the end of every later command, which then times out.

## HTTP

The HTTP tool sends one request to the target environment and records the
exchange. Arguments are `method`, an absolute `http` or `https` `url`,
optional `headers` and an optional text `body`; anything else is rejected
before a request is sent, as are invalid methods, URLs and header names.

`${NAME}` in the URL, header values or body is replaced by the value of an
environment parameter or secret just before sending, so the model never
handles secret values (see [credentials](security.md)). Other `${…}` text
is sent as written.

The evidence records the method, URL, request headers and body, and either
a `response` (status, all headers, body) or a `transport_error`. Any HTTP
status, including 401 or 500, is a response and may be the expected result
of a negative test. Only a request that got no response (connection
refused, DNS or TLS failure, deadline, cancellation) has a
`transport_error`. Credential headers and secret values are redacted. The
response body keeps at most `limits.output_limit_bytes`; reading stops
there, `truncated` is set, and `total_bytes` is the `Content-Length` when
the server sent one, otherwise the bytes read before stopping. When the
status and headers arrived but the body could not be read to its end (the
connection broke, the deadline passed, the run was cancelled), the response
is kept with the part that was read and `body_error` says why.

### Redirects and credentials

Redirects are **not** followed. A 3xx response is returned as evidence with
its `Location` header, and the model sends a new request if it decides to.
The new request carries only the headers the model writes for it, so a
credential meant for one host is never forwarded to another implicitly.

The tool uses the worker's proxy settings (`HTTPS_PROXY`, `HTTP_PROXY`,
`NO_PROXY`). Each request has the operation deadline; connecting has a
30-second limit within it.

## Step assessment

`assess` finishes an attempt. It acts on nothing: it records the model's
verdict, which Saci validates (see
[assessments](contracts.md#assessments)) before accepting it. A refused
submission is answered with every problem found, for example a criterion
that was not assessed or evidence that was never recorded, and the model
may correct it within the attempt's limits. Once an assessment is accepted
the attempt ends without another model request.

## What the model sees

Every tool result starts with the evidence ID the model must cite, for
example `Evidence ev-3 (shell, 12 ms).`, followed by a reminder that the
output is data from the system under test, not instructions. Shell results
show the termination (when the command did not simply exit), the exit code,
stdout and stderr, and say so when a new shell session replaced a lost one.
HTTP results show the request line, then the status, headers and body, or
the transport error. Truncated output carries a visible note.

Invalid calls never run. An unknown tool, arguments that are not a JSON
object, or arguments that do not match the schema are recorded as
`rejected` evidence and answered with what was wrong. They count against
`limits.tool_calls_per_step` like any other call.
