import { describe, expect, it } from "vitest";

import type { Attempt, RunReport, UnitReport } from "../../api/types";
import { describeAttempt } from "./PerformanceTab";
import { measure, share, slowest } from "./performance";

const at = (seconds: number) => new Date(Date.UTC(2026, 9, 21, 10, 0, seconds)).toISOString();

const attempt = (number: number, from: number, to: number | null, extra: Partial<Attempt> = {}): Attempt => ({
  number,
  started_at: at(from),
  finished_at: to === null ? null : at(to),
  outcome: to === null ? undefined : "passed",
  evidence: [],
  ...extra,
});

const unit = (key: string, phase: string, attempts: Attempt[]): UnitReport => ({
  key,
  id: key.split("#").pop() ?? key,
  kind: "step",
  title: `Unit ${key}`,
  phase,
  location: { document: "plan.md", line: 1 },
  cleanup: false,
  attempts,
});

const report = (units: UnitReport[], extra: Partial<RunReport> = {}): RunReport => ({
  schema_version: 1,
  run_id: "run-1",
  plan_title: "Plan",
  entry: "plan.md",
  environment: "local",
  policy: "default",
  model: "test-model",
  created_at: at(0),
  started_at: at(2),
  finished_at: at(62),
  lifecycle: "finished",
  counts: { passed: 0, failed: 0, blocked: 0, inconclusive: 0, error: 0, skipped: 0, cancelled: 0 },
  units,
  cleanup_not_run: [],
  evidence: [],
  last_seq: 1,
  ...extra,
});

const timing = (model_ms: number, tool_ms: number, model_requests = 2) => ({
  timing: { model_requests, model_ms, tool_ms, input_tokens: 1000, cached_tokens: 400, output_tokens: 50 },
});

describe("measure", () => {
  it("splits each attempt into model, tool and other time on the run's axis", () => {
    const run = measure(
      report([
        unit("1.1", "Setup", [attempt(1, 4, 14, timing(7000, 1000))]),
        unit("2.1", "Main", [attempt(1, 20, 50, timing(25_000, 2000))]),
      ]),
    );
    expect(run.span).toBe(60_000);
    const [setup, main] = run.phases;
    expect(setup?.units[0]?.attempts[0]).toMatchObject({ start: 2000, end: 12_000, model: 7000, tools: 1000 });
    expect(setup?.totals).toMatchObject({ duration: 10_000, model: 7000, tools: 1000, other: 2000, requests: 2 });
    expect(main).toMatchObject({ title: "Main", start: 18_000, end: 48_000 });
    expect(run.totals).toMatchObject({
      duration: 40_000,
      model: 32_000,
      tools: 3000,
      other: 5000,
      requests: 4,
      inputTokens: 2000,
      cachedTokens: 800,
      outputTokens: 100,
      untimed: 0,
    });
    expect(run.outside).toBe(20_000);
  });

  it("groups procedure units under the phase of the unit that invoked them", () => {
    const run = measure(
      report([
        unit("1.1>common/setup.md#1", "Procedure phase", [attempt(1, 3, 5)]),
        unit("1.1", "Setup", [attempt(1, 5, 8)]),
        unit("2.1", "Main", [attempt(1, 8, 9)]),
      ]),
    );
    expect(run.phases.map((p) => [p.title, p.units.map((u) => u.unit.key)])).toEqual([
      ["Setup", ["1.1>common/setup.md#1", "1.1"]],
      ["Main", ["2.1"]],
    ]);
  });

  it("counts every attempt of a retried unit", () => {
    const run = measure(
      report([unit("1.1", "Main", [attempt(1, 2, 12, timing(8000, 0)), attempt(2, 12, 17, timing(4000, 0))])]),
    );
    const entry = run.units[0];
    expect(entry?.attempts.map((a) => [a.start, a.end])).toEqual([
      [0, 10_000],
      [10_000, 15_000],
    ]);
    expect(entry?.totals).toMatchObject({ duration: 15_000, model: 12_000, attempts: 2 });
  });

  it("ends a running attempt and the run at the time of the measurement", () => {
    const now = Date.parse(at(30));
    const run = measure(
      report([unit("1.1", "Main", [attempt(1, 2, 10, timing(6000, 1000))]), unit("1.2", "Main", [attempt(1, 12, null)])], {
        finished_at: null,
        lifecycle: "running",
      }),
      now,
    );
    expect(run.span).toBe(28_000);
    const running = run.units[1]?.attempts[0];
    expect(running).toMatchObject({ start: 10_000, end: 28_000, running: true });
    expect(running?.model).toBeUndefined();
    expect(run.totals.untimed).toBe(1);
    expect(run.totals.model).toBe(6000);
  });

  it("keeps attempts without timing out of the model and tool totals", () => {
    const run = measure(report([unit("1.1", "Main", [attempt(1, 2, 12)])]));
    expect(run.totals).toMatchObject({ duration: 10_000, model: 0, tools: 0, other: 0, untimed: 1 });
  });

  it("leaves phases that did not run without a span", () => {
    const run = measure(report([unit("1.1", "Main", [attempt(1, 2, 4)]), unit("9.1", "Cleanup", [])]));
    expect(run.phases[1]?.title).toBe("Cleanup");
    expect(run.phases[1]?.start).toBeUndefined();
    expect(run.phases[1]?.end).toBeUndefined();
    expect(slowest(run, 10).map((u) => u.unit.key)).toEqual(["1.1"]);
  });
});

describe("slowest", () => {
  it("orders units by their total time, slowest first", () => {
    const run = measure(
      report([
        unit("1.1", "Main", [attempt(1, 2, 4)]),
        unit("1.2", "Main", [attempt(1, 4, 20)]),
        unit("1.3", "Main", [attempt(1, 20, 25)]),
      ]),
    );
    expect(slowest(run, 2).map((u) => u.unit.key)).toEqual(["1.2", "1.3"]);
  });
});

describe("share", () => {
  it("rounds to whole percentages and marks tiny parts", () => {
    expect(share(1, 3)).toBe("33%");
    expect(share(1, 1000)).toBe("<1%");
    expect(share(0, 1000)).toBe("0%");
    expect(share(5, 0)).toBe("0%");
  });
});

describe("describeAttempt", () => {
  it("says where a timed attempt spent its time", () => {
    const run = measure(report([unit("1.1", "Main", [attempt(1, 2, 12, timing(7000, 1500))])]));
    const span = run.units[0]?.attempts[0];
    expect(span && describeAttempt(span)).toBe("Attempt 1, passed, 10.0 s, model 7.0 s, tools 1.5 s, other 1.5 s");
  });
});
