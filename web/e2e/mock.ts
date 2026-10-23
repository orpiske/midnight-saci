// A stateful stand-in for saci-backend's client routes, answering the
// browser's requests to /api/v1 with the fixtures. Each test gets its own.
import type { Page, Request, Route } from "@playwright/test";

import type { Action, RunEvent, RunSummary } from "../src/api/types";
import * as fx from "./fixtures";

export interface Mock {
  /** Every API request, in order. */
  requests: Request[];
  actions: Action[];
  runs: RunSummary[];
  /** Answers the next submission with this instead of accepting it. */
  submit: ((route: Route, request: Request) => Promise<void> | void)[];
  /** Required bearer token, or null for an open backend. */
  token: string | null;
  events: RunEvent[];
  /** Console messages about content the Content-Security-Policy blocked. */
  blocked: string[];
}

function json(route: Route, status: number, body: unknown, headers: Record<string, string> = {}) {
  return route.fulfill({
    status,
    contentType: "application/json",
    headers,
    body: JSON.stringify(body),
  });
}

const error = (route: Route, status: number, code: string, message: string) =>
  json(route, status, { error: { code, message } });

export async function mockBackend(page: Page, options: Partial<Mock> = {}): Promise<Mock> {
  const mock: Mock = {
    requests: [],
    actions: [fx.pendingAction()],
    runs: [fx.liveRun, fx.failedRun],
    submit: [],
    token: null,
    events: fx.liveEvents,
    blocked: [],
    ...options,
  };
  page.on("console", (message) => {
    if (message.text().includes("Content Security Policy")) mock.blocked.push(message.text());
  });
  let served = 0;

  await page.route("**/api/v1/**", async (route, request) => {
    mock.requests.push(request);
    const url = new URL(request.url());
    const path = url.pathname.replace(/^\/api\/v1/, "");
    const method = request.method();
    if (path === "/health") return json(route, 200, { status: "ok" });
    if (mock.token && request.headers().authorization !== `Bearer ${mock.token}`) {
      return error(route, 401, "unauthorized", "send Authorization: Bearer <token>");
    }
    if (path === "/runs" && method === "GET") return json(route, 200, { items: mock.runs });
    if (path === "/runs" && method === "POST") {
      const answer = mock.submit.shift();
      if (answer) return answer(route, request);
      const accepted: RunSummary = { ...fx.liveRun, id: fx.LIVE };
      return json(route, 202, accepted, { location: `/api/v1/runs/${fx.LIVE}` });
    }
    if (path === "/environments") return json(route, 200, { items: fx.environments });
    if (path === "/policies") return json(route, 200, { items: fx.policies });
    if (path === "/actions") {
      const state = url.searchParams.get("state") ?? "pending_approval";
      return json(route, 200, { items: mock.actions.filter((a) => a.state === state) });
    }
    // The investigation of the failed run, and the run that executes it.
    if (path === `/investigations/${fx.INVESTIGATION}`) return json(route, 200, fx.investigation);
    if (path.startsWith("/investigations/")) return error(route, 404, "not_found", "investigation not found");
    if (path === `/runs/${fx.RUN}/investigations`) return json(route, 200, { items: [fx.investigation] });
    if (path.endsWith("/investigations")) return json(route, 200, { items: [] });
    if (path === `/runs/${fx.INVESTIGATION}/report`) return json(route, 200, fx.investigationReport);
    const run = /^\/runs\/([^/]+)(\/.*)?$/.exec(path);
    const summary = mock.runs.find((r) => r.id === decodeURIComponent(run?.[1] ?? ""));
    if (!run || !summary) return error(route, 404, "not_found", "no such run");
    const rest = run[2] ?? "";
    if (rest === "") return json(route, 200, summary);
    if (rest === "/report") {
      if (summary.id === fx.RUN) return json(route, 200, fx.report);
      const done = served >= mock.events.length;
      return json(route, 200, {
        ...fx.report,
        run_id: summary.id,
        plan_title: summary.plan_title,
        lifecycle: done ? "finished" : "running",
        outcome: done ? "passed" : undefined,
        units: [{ ...fx.report.units[0], outcome: done ? "passed" : undefined }],
        cleanup_not_run: [],
      });
    }
    if (rest === "/plan") return json(route, 200, { ...fx.plan, run_id: summary.id });
    if (rest === "/events") {
      const after = Number(url.searchParams.get("after") ?? "0");
      const items = summary.id === fx.LIVE ? mock.events.filter((e) => e.seq > after).slice(0, 2) : [];
      served = Math.max(served, ...items.map((e) => e.seq), 0);
      const finished = summary.id !== fx.LIVE || served >= mock.events.length;
      if (finished && summary.id === fx.LIVE) {
        summary.lifecycle = "finished";
        summary.outcome = "passed";
        summary.counts = { ...summary.counts, passed: 1 };
      }
      return json(route, 200, { items, next_after: items.at(-1)?.seq ?? after, finished });
    }
    if (rest === "/cancellation") {
      summary.cancel_requested_at = "2026-10-17T11:00:30Z";
      return json(route, 202, summary);
    }
    if (rest === "/actions") return json(route, 200, { items: mock.actions.filter((a) => a.run_id === summary.id) });
    const named = /^\/actions\/([^/]+)(?:\/(\w+))?$/.exec(rest);
    const action = mock.actions.find((a) => a.run_id === summary.id && a.name === named?.[1]);
    if (!named || !action) return error(route, 404, "not_found", "no such action");
    if (!named[2]) return json(route, 200, action);
    const body = (request.postDataJSON() ?? {}) as { digest?: string; note?: string };
    if (body.digest !== action.digest) {
      return error(route, 409, "digest_mismatch", "the digest does not match the recorded arguments");
    }
    if (named[2] === "approval" && action.state === "pending_approval") {
      action.state = "queued";
      action.decision = { verdict: "approved", at: "2026-10-17T10:05:00Z", note: body.note };
      return json(route, 200, action);
    }
    return error(route, 409, "action_not_pending", "the action is not awaiting approval");
  });
  return mock;
}
