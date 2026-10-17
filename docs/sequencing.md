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

Cleanup and teardown units (by title or phase title, see
[test plans](test-plans.md)) follow the same order and policy. They are not
run after an early stop, cancellation or the run deadline; the report lists
them under `cleanup_not_run`, so the target may still hold what the run
created. Destroying the worker does not clean up the target.

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
