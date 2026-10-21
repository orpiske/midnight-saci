// Where a run spent its time, computed from the report: per attempt, per
// unit and per phase, on one time axis that starts when the run started.
import type { Attempt, Outcome, RunReport, UnitReport } from "../../api/types";

/** One attempt on the time axis. Offsets are milliseconds from the origin. */
export interface AttemptSpan {
  number: number;
  start: number;
  end: number;
  /** The attempt has not finished; `end` is the time of the measurement. */
  running: boolean;
  outcome?: Outcome;
  /** Absent when the worker did not report timing for the attempt. */
  model?: number;
  tools?: number;
}

/** Time totals of a set of attempts. */
export interface Totals {
  /** Sum of the attempt durations. */
  duration: number;
  /** Model and tool time of the attempts that reported timing. */
  model: number;
  tools: number;
  /** Attempt time that is neither model nor tool time, for timed attempts. */
  other: number;
  requests: number;
  inputTokens: number;
  cachedTokens: number;
  outputTokens: number;
  attempts: number;
  /** Attempts without timing (an older worker image, or still running). */
  untimed: number;
}

export interface UnitTime {
  unit: UnitReport;
  /** Phase of the top-level unit that reached this unit. */
  phase: string;
  attempts: AttemptSpan[];
  totals: Totals;
}

export interface PhaseTime {
  title: string;
  /** First attempt start and last attempt end; absent when nothing ran. */
  start?: number;
  end?: number;
  units: UnitTime[];
  totals: Totals;
}

export interface RunTime {
  /** Epoch milliseconds of offset 0. */
  origin: number;
  /** Length of the time axis: the run time, or the time so far. */
  span: number;
  phases: PhaseTime[];
  units: UnitTime[];
  totals: Totals;
  /** Run time outside any attempt: worker start, sequencing, shutdown. */
  outside: number;
}

const time = (iso: string | null | undefined): number | undefined => {
  if (!iso) return undefined;
  const value = new Date(iso).getTime();
  return Number.isNaN(value) ? undefined : value;
};

function empty(): Totals {
  return {
    duration: 0,
    model: 0,
    tools: 0,
    other: 0,
    requests: 0,
    inputTokens: 0,
    cachedTokens: 0,
    outputTokens: 0,
    attempts: 0,
    untimed: 0,
  };
}

function add(into: Totals, from: Totals): void {
  into.duration += from.duration;
  into.model += from.model;
  into.tools += from.tools;
  into.other += from.other;
  into.requests += from.requests;
  into.inputTokens += from.inputTokens;
  into.cachedTokens += from.cachedTokens;
  into.outputTokens += from.outputTokens;
  into.attempts += from.attempts;
  into.untimed += from.untimed;
}

function attemptTotals(attempt: Attempt, span: AttemptSpan): Totals {
  const totals = empty();
  const duration = span.end - span.start;
  totals.duration = duration;
  totals.attempts = 1;
  const timing = attempt.timing;
  if (!timing || span.running) {
    totals.untimed = 1;
    return totals;
  }
  totals.model = timing.model_ms;
  totals.tools = timing.tool_ms;
  totals.other = Math.max(0, duration - timing.model_ms - timing.tool_ms);
  totals.requests = timing.model_requests;
  totals.inputTokens = timing.input_tokens ?? 0;
  totals.cachedTokens = timing.cached_tokens ?? 0;
  totals.outputTokens = timing.output_tokens ?? 0;
  return totals;
}

/** Measures the report at `now` (epoch milliseconds), which ends the
 * attempts and the run that have not finished. */
export function measure(report: RunReport, now: number = Date.now()): RunTime {
  const firstStart = report.units
    .flatMap((unit) => unit.attempts.map((attempt) => time(attempt.started_at)))
    .filter((value): value is number => value !== undefined);
  const origin =
    time(report.started_at) ?? (firstStart.length > 0 ? Math.min(...firstStart) : undefined) ?? time(report.created_at) ?? now;

  const byKey = new Map(report.units.map((unit) => [unit.key, unit]));
  const units: UnitTime[] = report.units.map((unit) => {
    const root = byKey.get(unit.key.split(">")[0] ?? unit.key) ?? unit;
    const totals = empty();
    const attempts: AttemptSpan[] = [];
    for (const attempt of unit.attempts) {
      const started = time(attempt.started_at);
      if (started === undefined) continue;
      const finished = time(attempt.finished_at);
      const running = finished === undefined;
      const span: AttemptSpan = {
        number: attempt.number,
        start: started - origin,
        end: Math.max(started, finished ?? now) - origin,
        running,
        outcome: attempt.outcome,
      };
      if (attempt.timing && !running) {
        span.model = attempt.timing.model_ms;
        span.tools = attempt.timing.tool_ms;
      }
      attempts.push(span);
      add(totals, attemptTotals(attempt, span));
    }
    return { unit, phase: root.phase, attempts, totals };
  });

  // Consecutive units of one phase form a group, as in the results view.
  const phases: PhaseTime[] = [];
  for (const entry of units) {
    let phase = phases[phases.length - 1];
    if (!phase || phase.title !== entry.phase) {
      phase = { title: entry.phase, units: [], totals: empty() };
      phases.push(phase);
    }
    phase.units.push(entry);
    add(phase.totals, entry.totals);
    for (const attempt of entry.attempts) {
      phase.start = Math.min(phase.start ?? attempt.start, attempt.start);
      phase.end = Math.max(phase.end ?? attempt.end, attempt.end);
    }
  }

  const totals = empty();
  for (const entry of units) add(totals, entry.totals);
  const lastEnd = Math.max(0, ...units.flatMap((entry) => entry.attempts.map((attempt) => attempt.end)));
  const finished = time(report.finished_at);
  const end = finished !== undefined ? finished - origin : report.lifecycle === "finished" ? lastEnd : now - origin;
  const span = Math.max(end, lastEnd, 0);
  return { origin, span, phases, units, totals, outside: Math.max(0, span - totals.duration) };
}

/** The units that took longest, slowest first; units that did not run are left out. */
export function slowest(run: RunTime, count: number): UnitTime[] {
  return run.units
    .filter((entry) => entry.attempts.length > 0)
    .sort((a, b) => b.totals.duration - a.totals.duration)
    .slice(0, count);
}

/** `part` as a whole-number percentage of `whole`. */
export function share(part: number, whole: number): string {
  if (whole <= 0) return "0%";
  const value = (part / whole) * 100;
  return value > 0 && value < 1 ? "<1%" : `${Math.round(value)}%`;
}
