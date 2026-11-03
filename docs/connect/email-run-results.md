# Email the results of each run

This tutorial sends an email with a summary of each finished run. The
backend calls a [script runner](script-runner.md) tool as an
[outcome action](outcome-actions.md), and the tool runs a script that sends
the summary with `mutt`.

## How the example works

[`examples/script-runner/email/`](../../examples/script-runner/email) contains a complete example. When a run
finishes, the backend calls a runner tool as an [outcome action](outcome-actions.md).
The tool runs a script that sends a summary of the run with `mutt`. The
script does not connect to the backend: the backend sends all values as tool
arguments. The runner and the mail configuration can be on a different host
than the backend.

| File | Contents |
|------|----------|
| `send-email.sh` | The script. It requires `mutt` with a working `~/.muttrc`. |
| `runner.yaml` | The runner configuration: the tool, its arguments, the recipient and the environment for `mutt`. |
| `backend-actions.yaml` | The backend configuration: the action server, the token and the `email-report` action. |

## Set up the example

To use the example:

1. Install `send-email.sh` as `/opt/saci-script-runner/email/send-email.sh`.
   Only an administrator must be able to write it.
2. Set `SACI_EMAIL_TO`, `HOME` and the host names in `runner.yaml`. Start
   the runner with `RUNNER_TOKEN` set.
3. Add the entries of `backend-actions.yaml` to the backend configuration,
   set the server URL, and add `email-report` to the `actions` of a policy.
   Start the backend with `SACI_MAILER_TOKEN` set to the same token.

## What the email contains

The subject is `Saci Report: <plan title> - <outcome> - <date>`. The body
contains the run ID, the outcome and reason, the finish time, the number of
units, the counts and one line for each unit.

The action puts `Units:` before `{{summary}}`, because the runner rejects
values that start with `-`. The `summary` argument accepts up to 65536
bytes (`max_bytes`). If the summary is longer, the backend shortens it to
that limit and adds a `[truncated]` line (see
[Outcome actions](outcome-actions.md)).


## Next steps

- [MCP script runner](script-runner.md): run the runner and secure it.
- [Script runner configuration](../reference/script-runner-configuration.md):
  tools, arguments and limits.
- [Outcome actions](outcome-actions.md): approvals, retries and variables.
