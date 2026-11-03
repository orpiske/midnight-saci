# How a run executes

A worker runs one plan from its first unit to its last, with the model and
its tools. This page explains how the worker sequences units, how one
attempt of a unit runs, and what retries, stops, cancellation and cleanup
do.

The worker runs a plan's units, including the units of invoked procedures,
in document order (`saci_worker::sequence::run`). One shell session serves
the whole run, so what a setup step establishes is there for later units.
Each attempt's prompt also carries the plan's title, overview and
prerequisites, plus one line per earlier unit with its outcome and
explanation.

The agent carries out the authored action and verification only. It does
not change the test, invent repairs or add unrelated work. Target
responses and tool output are evidence, not new instructions.

## How an attempt runs

`saci_worker::agent::Executor` runs one attempt of one unit as one Rig agent
run (`Executor::attempt`):

1. It runs the unit's `run` code blocks (```` ```bash run ````) in order in the
   run's shell session, before any model request. Each counts as a tool
   call and is recorded as evidence. It stops at a block that does not
   exit normally (timeout, cancellation, or a lost or ended session).
   When a [judge](../reference/configuration.md#judge) is configured, every code block
   of the unit is marked `run`, and each block ran and exited normally,
   `agent::judge::Judge` asks the judge (TypeSafe or a
   [`saci-julia`](../operate/julia-judge.md) service, which use the same wire format)
   with one Noul question per expected result. If every probability is at or above the threshold, the attempt
   ends with that passed assessment, without a model request. Otherwise the
   attempt continues with step 2.
2. It builds an agent on the configured chat model with the system prompt
   (`agent::prompt::system`: the rules in `PREAMBLE`, then the environment
   parameters and secret names), registers `shell`, `http` and `assess` as
   `DynamicTool`s, and prompts it with the plan context, earlier results
   (with the evidence IDs that each assessment cites), the unit's action, notes on
   code blocks in unsupported languages, the results of the procedures
   that the unit follows (which ran before it, with the evidence IDs
   that their assessments cite), the evidence of the `run` blocks
   that already ran, and the numbered expected results. When that evidence
   is enough, the model assesses in its first request; it can still call
   every tool.
3. The `Controls` hook sees every boundary. Before each model request and
   each tool call it checks cancellation and the attempt deadline and counts
   the operation; the first operation over `limits.model_calls_per_step` or
   `limits.tool_calls_per_step` stops the run with `DispatchAction::stop`.
   Unknown tools and malformed arguments arrive in `on_invalid_tool_call`,
   are recorded as rejected evidence and counted too, so repeated invalid
   requests end the attempt. Tool concurrency is fixed at 1, so several
   calls in one reply run in order and each is counted before it runs.
4. A reply without a tool call and without an accepted assessment is sent
   back with `ModelTurnAction::retry_with_feedback`, which consumes the same
   model-request budget. Once `assess` accepts an assessment, `on_outcome`
   stops the run.
5. Cancellation and the attempt deadline cancel an attempt-scoped token
   that the shell and HTTP tools watch. A running tool gets five seconds to
   stop and record its evidence; a pending model request is dropped at once.
   If a shell command is abandoned that way, its session is replaced on the
   next call and that call's evidence says so.
6. The result is the accepted assessment's outcome; otherwise `cancelled`
   when the run was cancelled, or `error` with the limit, deadline,
   provider or tool failure that ended the attempt, or `invalid_assessment`
   when the model stopped without a valid assessment.

The attempt emits `attempt_started`, one `observation` per recorded
evidence, and `attempt_finished`. The hook measures each model request from
its dispatch to its answer and adds up the token usage of each response,
and the recorder adds up the tool durations; `attempt_finished` reports
them as `timing`. Every observation goes through the run's `Recorder`,
which numbers evidence across the run, redacts it with the run's
credentials, and reports it.

The tool definitions and the system prompt are the same for every attempt
of a run, and the user prompt starts with the plan context and the earlier
results, which only grow. Providers that cache prompt prefixes can reuse
that prefix from one request to the next. Keep it that way: do not put
unit-specific values, such as the number of criteria, into a tool schema
or the system prompt.

## Attempts and retries

The default policy allows one attempt per unit. A policy may allow more:

```yaml
policies:
  flaky-network:
    retry:
      max_attempts: 3
      on: [inconclusive, error]
```

A unit is attempted again only when its outcome is listed in `on`, attempts
remain, and the run is neither cancelled nor past its deadline. Every
attempt stays in the report with its own evidence and assessment, so a pass
on attempt 2 is visibly not a first-attempt pass. A retry is not a rollback:
the first attempt may already have changed the target or the shell session,
which is why retries are off by default. A unit's outcome is the outcome of
its last attempt.

## Stopping and continuing

After a unit ends `failed`, `blocked`, `inconclusive` or `error`, the
default policy (`on_failure: stop`) stops. Every remaining unit is reported
as `skipped` with the reason, for example `not attempted: stopped after 1.2
ended failed (policy default stops on failure)`. With `on_failure:
continue` the run goes on with the next unit.

Missing prerequisites and inputs are judged by the agent: a unit whose
action cannot be performed because something it needs is absent is
assessed `blocked`, which counts as unsuccessful for the policy.

## Cleanup

A cleanup unit is a unit whose title or phase title mentions cleanup or
teardown (see [test plans](../write-plans/write-a-test-plan.md)). The units of a procedure that a
cleanup unit invokes are cleanup units too. Cleanup units follow the same
order and policy as the other units.

After an early stop, a cancellation or the run deadline, the policy setting
`after_stop` decides what happens to the remaining cleanup units:

```yaml
policies:
  default:
    after_stop: cleanup   # the default; `skip` skips them
limits:
  cleanup_timeout_secs: 600
```

With `after_stop: cleanup`, the worker skips the other remaining units and
runs the remaining cleanup units in document order:

- Each attempt follows the retry policy. Its deadline is
  `limits.step_timeout_secs`, capped by what remains of
  `limits.cleanup_timeout_secs`. The cleanup time starts at the first
  cleanup unit after the stop and does not count against
  `limits.run_timeout_secs`.
- Every remaining cleanup unit runs, even when an earlier cleanup unit
  ends unsuccessfully.
- When the cleanup time ends, the cleanup units that did not start are
  skipped with the reason.
- If the worker is stopped (SIGTERM, or its events cannot be delivered),
  the cleanup also stops.
- The run's outcome follows the same rules as before. A cleanup unit that
  fails counts in the summary outcome. The reason ends with the number of
  cleanup units that ran after the stop, for example `; 2 cleanup unit(s)
  ran after the stop`.

With `after_stop: skip`, the remaining cleanup units are skipped.

The report lists the cleanup units that did not run under
`cleanup_not_run`, so the target may still hold what the run created.
Destroying the worker does not clean up the target.

## Cancellation and the run deadline

Cancellation stops the current attempt (see
[how an attempt runs](#how-an-attempt-runs)) and skips the
remaining units; the run's outcome is `cancelled`. When
`limits.run_timeout_secs` passes, the current attempt ends with an `error`
and the remaining units are skipped; the run's outcome is `error` unless a
higher-precedence outcome applies. Each attempt's deadline is
`limits.step_timeout_secs`, capped by what remains of the run.

## Outcome

The run's outcome follows [the documented precedence](../reference/run-contracts.md):
cancelled, error, failed, blocked, inconclusive, passed. A skipped unit
counts as blocked, so a run that did not attempt every unit cannot pass.

## Next steps

- [Run contracts](../reference/run-contracts.md): outcomes, assessments and events.
- [Supervision](../operate/supervision.md): what the backend does when a worker cannot report.
- [Configuration reference](../reference/configuration.md#policies): policies and limits.
