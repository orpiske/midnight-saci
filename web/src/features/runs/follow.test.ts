import { describe, expect, it } from "vitest";

import { ApiError, NetworkError } from "../../api/client";
import type { EventPage, RunEvent } from "../../api/types";
import { type FollowState, continuation, follow } from "./follow";

function event(seq: number, type: RunEvent["type"] = "attempt_started"): RunEvent {
  const base = { run_id: "run-a", seq, at: "2026-10-18T00:00:00Z" };
  if (type === "run_finished") return { ...base, type, outcome: "passed", reason: "done" };
  return { ...base, type: "attempt_started", unit: "1.1", attempt: 1 };
}

/** A backend holding `total` events of a finished run, paged by `limit`. */
function finishedRun(total: number, limit: number) {
  const all = Array.from({ length: total }, (_, i) =>
    event(i + 1, i + 1 === total ? "run_finished" : "attempt_started"),
  );
  const calls: { after: number; wait: number }[] = [];
  const fetchPage = async (after: number, wait: number): Promise<EventPage> => {
    calls.push({ after, wait });
    const items = all.filter((e) => e.seq > after).slice(0, limit);
    return { items, next_after: items.at(-1)?.seq ?? after, finished: true };
  };
  return { fetchPage, calls };
}

async function collect(fetchPage: (after: number, wait: number) => Promise<EventPage>) {
  const seen: number[] = [];
  const states: FollowState[] = [];
  let finished = 0;
  await follow(
    {
      fetchPage,
      onEvents: (events) => seen.push(...events.map((e) => e.seq)),
      onState: (state) => states.push(state),
      onFinished: () => finished++,
      sleep: async () => {},
    },
    new AbortController().signal,
  );
  return { seen, states, finished };
}

describe("following a run", () => {
  it("drains a finished run with more events than one page", async () => {
    const backend = finishedRun(137, 50);
    const { seen, finished } = await collect(backend.fetchPage);
    expect(seen).toEqual(Array.from({ length: 137 }, (_, i) => i + 1));
    expect(finished).toBe(1);
    expect(backend.calls.map((c) => c.after)).toEqual([0, 50, 100]);
    expect(backend.calls.slice(1).every((c) => c.wait === 0)).toBe(true);
  });

  it("shows every event once across a disconnect and repeated pages", async () => {
    const pages: (() => Promise<EventPage>)[] = [
      async () => ({ items: [event(1), event(2)], next_after: 2, finished: false }),
      async () => Promise.reject(new NetworkError("dropped")),
      async () => ({ items: [event(2), event(3)], next_after: 3, finished: false }),
      async () => ({ items: [event(4, "run_finished")], next_after: 4, finished: true }),
    ];
    const afters: number[] = [];
    const { seen, states, finished } = await collect(async (after) => {
      afters.push(after);
      const next = pages.shift();
      if (!next) throw new Error("polled after the end");
      return next();
    });
    expect(seen).toEqual([1, 2, 3, 4]);
    expect(afters).toEqual([0, 2, 2, 3]);
    expect(states.map((s) => s.status)).toEqual(["live", "reconnecting", "live", "done"]);
    expect(finished).toBe(1);
  });

  it("ends on an empty finished page when the log has no terminal event", async () => {
    const pages: EventPage[] = [
      { items: [event(1)], next_after: 1, finished: true },
      { items: [], next_after: 1, finished: true },
    ];
    const { seen, finished } = await collect(async () => pages.shift() as EventPage);
    expect(seen).toEqual([1]);
    expect(finished).toBe(1);
  });

  it("stops on a refusal instead of retrying", async () => {
    const { states, finished } = await collect(async () => {
      throw new ApiError(404, "not_found", "run not found");
    });
    expect(states.map((s) => s.status)).toEqual(["stopped"]);
    expect(finished).toBe(0);
  });

  it("stops polling once aborted", async () => {
    const controller = new AbortController();
    let polls = 0;
    await follow(
      {
        fetchPage: async () => {
          polls++;
          controller.abort();
          return { items: [event(1)], next_after: 1, finished: false };
        },
        onEvents: () => {
          throw new Error("delivered after abort");
        },
        onState: () => {},
        onFinished: () => {},
      },
      controller.signal,
    );
    expect(polls).toBe(1);
  });

  it("accepts only the contiguous continuation", () => {
    expect(continuation(2, [event(5), event(3), event(2)]).map((e) => e.seq)).toEqual([3]);
  });
});
