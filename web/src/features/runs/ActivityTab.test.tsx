import { describe, expect, it } from "vitest";

import type { RunEvent } from "../../api/types";
import { describeEvent } from "./ActivityTab";

const finished = (
  extra: Partial<Extract<RunEvent, { type: "attempt_finished" }>>,
): RunEvent => ({
  run_id: "run-1",
  seq: 4,
  at: "2026-10-20T08:00:00Z",
  unit: "1.1",
  attempt: 1,
  type: "attempt_finished",
  outcome: "passed",
  ...extra,
});

describe("describeEvent", () => {
  it("shows where a finished attempt spent its time", () => {
    const event = finished({
      timing: { model_requests: 2, model_ms: 3120, tool_ms: 12 },
    });
    expect(describeEvent(event).text).toBe(
      "Attempt 1 passed (model: 2 requests, 3120 ms; tools: 12 ms)",
    );
  });

  it("leaves the timing out when the worker did not report it", () => {
    expect(describeEvent(finished({})).text).toBe("Attempt 1 passed");
  });
});
