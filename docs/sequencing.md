# Run sequencing and policy

The worker runs a plan's units, including the units of invoked procedures,
in document order (`saci_worker::sequence::run`). One shell session serves
the whole run, so what a setup step establishes is there for later units.
Each attempt's prompt also carries the plan's title, overview and
prerequisites, plus one line per earlier unit with its outcome and
explanation.

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
teardown (see [test plans](test-plans.md)). The units of a procedure that a
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
[agent execution](agent-library.md#how-an-attempt-runs)) and skips the
remaining units; the run's outcome is `cancelled`. When
`limits.run_timeout_secs` passes, the current attempt ends with an `error`
and the remaining units are skipped; the run's outcome is `error` unless a
higher-precedence outcome applies. Each attempt's deadline is
`limits.step_timeout_secs`, capped by what remains of the run.

## Outcome

The run's outcome follows [the documented precedence](contracts.md):
cancelled, error, failed, blocked, inconclusive, passed. A skipped unit
counts as blocked, so a run that did not attempt every unit cannot pass.
