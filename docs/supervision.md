# Supervising runs

The backend owns every run from submission to its stored outcome. Once a
run is accepted it launches the run's worker
([launchers](launchers.md)), watches it, stops it when the run is cancelled
or overruns its deadline, records an outcome when the worker cannot, and
removes the worker afterwards. Every run that reaches a worker therefore
ends with a report: the worker's own `run_finished`, or one the backend
records on its behalf.

## The life of a run

1. **Launch.** The backend issues the run's worker token and starts the
   worker named `saci-<run id>`. When the launch fails, the run ends
   `error` with the runtime's reason, for example
   `the worker could not be launched: ...`.
2. **Watch.** Every 2 seconds the backend checks the run record and the
   worker's state.
3. **End.** The run ends when the worker reports `run_finished`, or when
   the backend records an outcome because the worker is gone (see below).
4. **Clean up.** After the run ends, the backend gives the worker 30
   seconds to exit by itself. Then it stops the worker if it is still
   running and removes it. Nothing a run created outlives it. Removal
   failures are logged and do not change the outcome.

## Uncertain launches

A launch can time out or lose its answer while the runtime still creates
the worker. In that case the backend asks the runtime whether `saci-<run
id>` exists:

- If it exists, it is watched like any other worker.
- If it certainly does not exist, the launch is repeated once.
- A name the runtime reports as already in use counts as launched.

The stable name means a repeated launch can never create a second worker.

## Cancellation

`POST /runs/{run}/cancellation` (or `saci cancel`) records the request.
The watcher then stops the worker with SIGTERM, followed by SIGKILL after
30 seconds. On SIGTERM the worker cancels its running step, keeps the
evidence it collected, and reports the run as `cancelled`. If the worker
dies before reporting, the backend records `cancelled` with the reason
`the run was cancelled; its worker stopped before reporting the end`.

## Deadlines

The worker enforces `limits.run_timeout_secs` itself and reports the run
as timed out. The backend is the backstop: when a run is still unfinished
60 seconds past `created_at + run_timeout_secs`, the backend stops the
worker and records `error` with the reason `the run exceeded its deadline
(limits.run_timeout_secs) and its worker was stopped`.

## Lost workers

A worker that exits or disappears without reporting its end leaves an
unfinished run. The backend keeps every event the worker had delivered,
then ends the run `error` with one of these reasons:

| What happened | Reason |
| --- | --- |
| The worker exited | `the worker exited with code N before reporting the end`, followed by its last output lines, redacted with the run's secrets |
| The runtime no longer knows the worker | `the worker disappeared before reporting the end` |

## Restarting the backend

When the backend starts, it looks at every run that is not finished yet,
before it accepts new launches:

| The run | What the backend does |
| --- | --- |
| Its worker still exists | Watches it again. The worker retries its reports while the backend is down, so its progress and end arrive once the backend is back. |
| It was accepted but never handed to a worker (no worker token was issued) | Launches it now. Nothing has run yet. |
| Anything else (a token was issued, but the worker is gone) | Ends it `error` with `the backend restarted and the run's worker is gone; the run is not repeated because its actions may already have run`. |

The backend never repeats a run that may already have acted on its
targets. Resubmit such a run yourself if repeating it is safe.

Workers of runs that had already finished, whose cleanup the earlier
process did not complete, are removed. Workers of runs this backend does
not know are left alone, since another backend may own them.

When no runtime is usable at start-up, every unfinished run ends `error`
with the launch failure.

## Checked by

- `crates/saci-backend/tests/supervision.rs` uses a fake runtime to cover
  uncertain launches, cancellation, deadlines, exits with and without a
  report, disappearance, cleanup, each recovery case and leftover removal.
- `crates/saci-backend/tests/container_launcher.rs` uses Docker or Podman
  and the real image (`SACI_WORKER_IMAGE`). It launches a worker that runs
  a plan to the end, and cancels one in the middle of `sleep 60`, which
  ends `cancelled` with the step's evidence. Both containers are removed.
