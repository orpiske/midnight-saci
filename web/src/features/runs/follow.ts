// Following a run's events with the backend's long poll
// (docs/api.md "Following progress"). Events arrive once each, in
// sequence order, across reconnects. A finished run is drained page by
// page until its terminal event; `finished` alone never ends the backlog.
import { ApiError } from "../../api/client";
import type { EventPage, RunEvent } from "../../api/types";

export type FollowStatus = "live" | "reconnecting" | "done" | "stopped";

export interface FollowState {
  status: FollowStatus;
  /** When the backend last answered a poll. */
  lastUpdate: Date | null;
  error?: unknown;
}

export interface FollowOptions {
  fetchPage: (after: number, wait: number, signal: AbortSignal) => Promise<EventPage>;
  onEvents: (events: RunEvent[]) => void;
  onState: (state: FollowState) => void;
  /** The run ended: fetch the authoritative report now. */
  onFinished: () => void;
  after?: number;
  /** Long-poll wait in seconds while the run is active. */
  wait?: number;
  /** Delay before reconnect attempt `n` (1-based), in milliseconds. */
  backoff?: (attempt: number) => number;
  sleep?: (ms: number, signal: AbortSignal) => Promise<void>;
  now?: () => Date;
}

const defaultBackoff = (attempt: number): number => Math.min(15_000, 1000 * 2 ** (attempt - 1));

function defaultSleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal.addEventListener(
      "abort",
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true },
    );
  });
}

/** The events of `page` that continue the sequence after `after`, without
 * repeats; it stops at a gap, which the next poll fills. */
export function continuation(after: number, items: readonly RunEvent[]): RunEvent[] {
  const accepted: RunEvent[] = [];
  let last = after;
  for (const event of [...items].sort((a, b) => a.seq - b.seq)) {
    if (event.seq <= last) continue;
    if (event.seq !== last + 1) break;
    accepted.push(event);
    last = event.seq;
  }
  return accepted;
}

/** Follows until the run ends, a permanent error, or `signal` aborts. */
export async function follow(options: FollowOptions, signal: AbortSignal): Promise<void> {
  const wait = options.wait ?? 25;
  const backoff = options.backoff ?? defaultBackoff;
  const sleep = options.sleep ?? defaultSleep;
  const now = options.now ?? (() => new Date());
  let after = options.after ?? 0;
  let draining = false;
  let failures = 0;
  let lastUpdate: Date | null = null;

  while (!signal.aborted) {
    let page: EventPage;
    try {
      page = await options.fetchPage(after, draining ? 0 : wait, signal);
    } catch (error) {
      if (signal.aborted) return;
      if (error instanceof ApiError && error.status < 500) {
        options.onState({ status: "stopped", lastUpdate, error });
        return;
      }
      failures += 1;
      options.onState({ status: "reconnecting", lastUpdate, error });
      await sleep(backoff(failures), signal);
      continue;
    }
    if (signal.aborted) return;
    failures = 0;
    lastUpdate = now();
    const fresh = continuation(after, page.items);
    const last = fresh[fresh.length - 1];
    if (last) {
      after = last.seq;
      options.onEvents(fresh);
    }
    const ended = fresh.some((event) => event.type === "run_finished");
    if (ended || (page.finished && fresh.length === 0)) {
      options.onState({ status: "done", lastUpdate });
      options.onFinished();
      return;
    }
    options.onState({ status: "live", lastUpdate });
    draining = page.finished;
  }
}
