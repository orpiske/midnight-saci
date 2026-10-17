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
