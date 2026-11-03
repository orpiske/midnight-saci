# Write a test plan

A Saci test plan is a Markdown document that you write the way you would
for a colleague: phases, numbered tests and steps, prose instructions,
fenced commands and the result you expect to see. This page tells you how
to write a plan that Saci can run and assess. The exact rules are in the
[test plan format](../reference/test-plan-format.md).

## Create a plan

```sh
saci new test-plan saci-on-the-cloud.md
```

The command writes an editable starter based on the
[starter template](../templates/test-plan.md). A complete example is next
to it in [templates/example.md](../templates/example.md). The command works
offline and never overwrites a file (see
[`saci new test-plan`](../reference/cli.md#saci-new-test-plan)). Edit the
overview, prerequisites, actions and expected results for your system
before you submit a run.

## Write actions and expected results

Prefer the exact command, the way you would run it by hand: a fenced `curl`
reproduces the step outside Saci, and its output is the evidence. Write
actions in prose, fenced commands, or both. Prose can tell the worker to use
its HTTP tool or an enabled MCP tool. Use a fenced block with a shell
language such as `bash` for commands. Commands that you write are
authoritative: the agent does not replace a failing command with a
different test.

For example, inside a test:

````markdown
### Test 1.1: List MCP tools

Run:

```bash
wanaku mcp tool list --uri "${MCP_SERVER_URI}" --plain
```

**Expected result:**

- The command exits with code 0.
- The output includes the `echo` tool.
````

Every test and step must have an explicit expected result, also setup and
cleanup steps. Describe observable outcomes, such as a response status, an
output value or a resource condition. If you list several criteria, all of
them must hold for the unit to pass. When more than one outcome is
acceptable, state the alternatives.

A successful tool call, a zero exit code or a printed `PASS` message alone
does not show that all criteria hold. The worker records the observations
that support its verdict. If the evidence cannot establish a verdict, the
result is inconclusive, with a reason. The agent does not invent missing
success criteria.

## Mark blocks that run as written

Add `run` after the language of a block that the step runs exactly as
written, every time:

````markdown
```bash run
wanaku mcp tool list --uri "${MCP_SERVER_URI}" --plain
```
````

The worker runs these blocks before it asks the model, so a `run` block
saves one model request for each unit, and the model cannot change the
command. With a [judge](../reference/configuration.md#judge), a unit whose
blocks are all marked `run` can pass without a model request. See
[run blocks](../reference/test-plan-format.md#run-blocks) for the exact
behavior.

Do not mark a block that runs only under a condition, a block that the
action uses as data, or a block that must run as a separate script. Leave
these blocks unmarked: the model runs them when the action tells it to.

## Prerequisites, setup, and cleanup

Prerequisites describe conditions that must hold before execution:
installed tools, available credentials, existing resources and required
inputs. A missing prerequisite blocks the affected work. It does not let the
agent install software or invent setup procedures.

Put the actions that establish conditions into explicit setup steps. For
example, to create a test resource is a step; to have credentials to create
it is a prerequisite. Put teardown actions into explicit cleanup steps with
their own expected results, and mention cleanup or teardown in the step or
phase title. Under the default policy, these steps also run after an early
stop, a cancellation or the run deadline (see
[cleanup](../run-tests/how-a-run-executes.md#cleanup)). Thus write each
cleanup step so that it works when earlier steps did not run or did not
finish: for example, delete a namespace only if it exists.

## Inputs and settings

Document the names and meanings of required inputs in the plan. Supply the
actual endpoints, credentials, shared parameters, enabled tools, timeouts
and limits through the [environment](../reference/configuration.md#environments)
and the run policy. Credentials are secret references in the configuration,
never values in Markdown.

In shell commands, `${VARIABLE}` uses normal shell expansion. Saci has no
second interpolation language for Markdown. Prose refers to inputs by name,
for example: "Send an HTTP GET request to `SERVICE_URL` followed by
`/health`."

One shell session serves the whole run. Files, variables, helper functions
and the working directory that a step sets up are there for later steps
(see [worker tools](../reference/worker-tools.md#shell)). This state is
private to the run.

Failure actions, approval rules and retries belong to the run policy. Plans
cannot grant tools, add MCP servers, change the policy or override the
execution limits.

To use one set of steps in several plans, put them in a procedure (see
[reuse steps with procedures](reusable-procedures.md)).

## Check the plan

```sh
saci check plan.md
```

`saci check` validates the plan and its procedures offline, without a
backend, with the same rules as `saci run`. Each error gives the file and
line. It also warns about content that does not run, for example a phase
without tests or steps, or a `Follow` outside a unit. See
[`saci check`](../reference/cli.md#saci-check).

## Next steps

- [Test plan format](../reference/test-plan-format.md): the exact parser
  rules.
- [Reuse steps with procedures](reusable-procedures.md).
- [Run and follow plans](../run-tests/run-and-follow.md).
