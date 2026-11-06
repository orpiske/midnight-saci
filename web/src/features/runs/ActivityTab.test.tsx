import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { RunEvent } from "../../api/types";
import { IN_PROGRESS } from "../../lib/status";
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

describe("describeEvent token counts", () => {
  it("shows the tokens the provider counted, cached ones included", () => {
    const event = finished({
      timing: {
        model_requests: 2,
        model_ms: 3120,
        tool_ms: 12,
        input_tokens: 5200,
        cached_tokens: 4096,
        output_tokens: 310,
      },
    });
    expect(describeEvent(event).text).toBe(
      "Attempt 1 passed (model: 2 requests, 3120 ms, tokens: 5200 in (4096 cached), 310 out; tools: 12 ms)",
    );
  });
});

describe("describeEvent running commands", () => {
  it("shows the first line of a shell command that is still running", () => {
    const event: RunEvent = {
      run_id: "run-1",
      seq: 3,
      at: "2026-10-20T08:00:00Z",
      unit: "1.1",
      attempt: 1,
      type: "command_running",
      command: 'kill "$PID"\nwait "$PID"',
      elapsed_secs: 30,
    };
    const { look, text } = describeEvent(event);
    const { container } = render(<>{text}</>);
    expect(container.textContent).toBe('Shell still running after 30 s: kill "$PID"');
    expect(look).toBe(IN_PROGRESS);
  });
});
