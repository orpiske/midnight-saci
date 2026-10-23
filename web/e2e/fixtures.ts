// API answers for the browser tests, typed with the generated OpenAPI
// types so they cannot drift from the contract.
import type {
  Action,
  EnvironmentInfo,
  Evidence,
  Investigation,
  PolicyInfo,
  RunEvent,
  RunPlan,
  RunReport,
  RunSummary,
} from "../src/api/types";

export const RUN = "run-3f1c9a2b7d4e4f0a9c1b2d3e4f5a6b7c";
export const LIVE = "run-0f3c9a2b7d4e4f1a9b8c7d6e5f4a3b2c";

const counts = { passed: 0, failed: 0, blocked: 0, inconclusive: 0, error: 0, skipped: 0, cancelled: 0 };

export const failedRun: RunSummary = {
  id: RUN,
  plan_title: "Service health",
  environment: "local",
  policy: "review",
  lifecycle: "finished",
  outcome: "failed",
  reason: "Test 1.2 failed",
  counts: { ...counts, passed: 1, failed: 1 },
  total_units: 3,
  last_event_seq: 12,
  created_at: "2026-10-17T10:00:00Z",
  finished_at: "2026-10-17T10:01:30Z",
};

export const liveRun: RunSummary = {
  id: LIVE,
  plan_title: "Checkout flow",
  environment: "local",
  policy: "default",
  lifecycle: "running",
  current_unit: "1.1",
  counts,
  total_units: 1,
  last_event_seq: 1,
  created_at: "2026-10-17T11:00:00Z",
};

const at = "2026-10-17T10:00:10Z";

export const evidence: Evidence[] = [
  {
    id: "ev-1",
    unit: "1.1",
    attempt: 1,
    recorded_at: at,
    duration_ms: 120,
    tool: "shell",
    command: "curl -s -o /dev/null -w '%{http_code}' $URL/health",
    stdout: { text: "200", total_bytes: 3 },
    stderr: { text: "", total_bytes: 0 },
    exit_code: 0,
    termination: "exited",
  },
  {
    id: "ev-2",
    unit: "1.2",
    attempt: 1,
    recorded_at: at,
    duration_ms: 340,
    tool: "http",
    method: "GET",
    url: "http://service.internal/orders",
    request_headers: { authorization: "Bearer [REDACTED:TARGET_TOKEN]" },
    response: {
      status: 500,
      headers: { "content-type": "text/plain" },
      body: { text: "internal error\u001b[31m boom", total_bytes: 26 },
    },
  },
];

const criterion = (index: number, text: string, line: number) => ({
  index,
  text,
  location: { document: "plan.md", line },
});

export const report: RunReport = {
  schema_version: 1,
  run_id: RUN,
  plan_title: "Service health",
  entry: "plan.md",
  environment: "local",
  policy: "review",
  model: "test-model",
  backend_build: { version: "0.3.0", commit: "1e81188c0a5f1ba58741b9d6e5f4a3b2c1d0e9f8" },
  worker_environment: {
    saci: { version: "0.3.0", commit: "1e81188c0a5f1ba58741b9d6e5f4a3b2c1d0e9f8" },
    os: "Fedora Linux 44 (Container Image)",
    kernel: "Linux 6.12.0",
    arch: "x86_64",
    tools: [
      { name: "bash", version: "GNU bash, version 5.2.37(1)-release (x86_64-redhat-linux-gnu)" },
      { name: "curl", version: "curl 8.15.0 (x86_64-redhat-linux-gnu)" },
    ],
  },
  worker_image: {
    reference: "quay.io/saci/saci-worker:0.3.0",
    digest: "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
    id: "sha256:4b2f8e0c1d3a5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f",
  },
  created_at: "2026-10-17T10:00:00Z",
  lifecycle: "finished",
  outcome: "failed",
  reason: "Test 1.2 failed",
  started_at: "2026-10-17T10:00:02Z",
  finished_at: "2026-10-17T10:01:30Z",
  counts: { ...counts, passed: 1, failed: 1 },
  units: [
    {
      key: "1.1",
      id: "1.1",
      kind: "test",
      title: "Check health",
      phase: "Main",
      location: { document: "plan.md", line: 5 },
      cleanup: false,
      criteria: [criterion(0, "The status is 200.", 11)],
      outcome: "passed",
      attempts: [
        {
          number: 1,
          started_at: "2026-10-17T10:00:03Z",
          finished_at: "2026-10-17T10:00:12Z",
          timing: { model_requests: 2, model_ms: 6400, tool_ms: 900, input_tokens: 5200, cached_tokens: 4096, output_tokens: 310 },
          outcome: "passed",
          evidence: ["ev-1"],
          assessment: {
            outcome: "passed",
            explanation: "The health endpoint answered 200.",
            criteria: [
              {
                criterion: 0,
                status: "satisfied",
                basis: "mechanical",
                evidence: ["ev-1"],
                explanation: "ev-1 printed 200 and exited 0.",
              },
            ],
          },
        },
      ],
    },
    {
      key: "1.2",
      id: "1.2",
      kind: "test",
      title: "List orders",
      phase: "Main",
      location: { document: "plan.md", line: 13 },
      cleanup: false,
      criteria: [criterion(0, "The orders endpoint answers 200.", 19)],
      outcome: "failed",
      attempts: [
        {
          number: 1,
          started_at: "2026-10-17T10:00:20Z",
          finished_at: "2026-10-17T10:00:40Z",
          timing: { model_requests: 3, model_ms: 16200, tool_ms: 2100, input_tokens: 8100, output_tokens: 620 },
          outcome: "failed",
          evidence: ["ev-2"],
          assessment: {
            outcome: "failed",
            explanation: "The endpoint answered 500.",
            criteria: [
              {
                criterion: 0,
                status: "contradicted",
                basis: "mechanical",
                evidence: ["ev-2"],
                explanation: "ev-2 has status 500.",
              },
            ],
          },
        },
      ],
    },
    {
      key: "2.1",
      id: "2.1",
      kind: "step",
      title: "Remove the data",
      phase: "Cleanup",
      location: { document: "plan.md", line: 23 },
      cleanup: true,
      criteria: [criterion(0, "Nothing remains.", 29)],
      outcome: "skipped",
      reason: "the run stopped after Test 1.2 failed",
      attempts: [],
    },
  ],
  cleanup_not_run: ["2.1"],
  evidence,
  last_seq: 12,
};

export const PLAN_SOURCE = `# Test Plan: Service health

## Phase 1: Main

### Test 1.1: Check health

Run \`curl $URL/health\`.

**Expected result:**

- The status is 200.
`;

export const plan: RunPlan = {
  run_id: RUN,
  title: "Service health",
  entry: "plan.md",
  documents: { "plan.md": PLAN_SOURCE },
  units: report.units.map((unit) => ({
    key: unit.key,
    id: unit.id,
    kind: unit.kind,
    title: unit.title,
    phase: unit.phase,
    location: unit.location,
    cleanup: unit.cleanup,
    action: `Run the check for ${unit.title.toLowerCase()}.`,
    criteria: unit.criteria ?? [],
  })),
};

export const environments: EnvironmentInfo[] = [
  {
    name: "local",
    description: "Laptop services",
    limits: {
      run_timeout_secs: 3600,
      step_timeout_secs: 300,
      operation_timeout_secs: 60,
      model_request_timeout_secs: 120,
      model_request_retries: 2,
      model_calls_per_step: 20,
      tool_calls_per_step: 40,
      output_limit_bytes: 65536,
    },
  },
];

export const policies: PolicyInfo[] = [
  { name: "default", configured: false, on_failure: "stop", retry: { max_attempts: 1 }, actions: [] },
  {
    name: "review",
    configured: true,
    on_failure: "continue",
    retry: { max_attempts: 2, on: ["failed"] },
    actions: [{ name: "escalate", server: "tickets", tool: "record", on: ["failed"], approval: "required" }],
  },
];

export function pendingAction(): Action {
  return {
    run_id: RUN,
    name: "escalate",
    server: "tickets",
    tool: "record",
    approval: "required",
    state: "pending_approval",
    arguments: { title: `Escalate ${RUN}`, priority: "high" } as unknown as Action["arguments"],
    digest: "9f2c4e1a7b3d5f6081a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f7081",
    attempts: 0,
    created_at: "2026-10-17T10:01:31Z",
    updated_at: "2026-10-17T10:01:31Z",
    history: [
      { at: "2026-10-17T10:01:31Z", event: "recorded", state: "pending_approval", attempt: 0 },
    ],
  };
}

type Body = RunEvent extends infer E ? (E extends RunEvent ? Omit<E, "run_id" | "seq" | "at"> : never) : never;

const event = (seq: number, rest: Body): RunEvent =>
  ({ run_id: LIVE, seq, at: `2026-10-17T11:00:${String(seq).padStart(2, "0")}Z`, ...rest }) as RunEvent;

export const liveEvents: RunEvent[] = [
  event(1, { type: "run_started", model: "test-model" }),
  event(2, { type: "attempt_started", unit: "1.1", attempt: 1 }),
  event(3, {
    type: "observation",
    unit: "1.1",
    attempt: 1,
    evidence: { ...evidence[0], unit: "1.1" } as Evidence,
  }),
  event(4, { type: "attempt_finished", unit: "1.1", attempt: 1, outcome: "passed" }),
  event(5, { type: "unit_finished", unit: "1.1", outcome: "passed" }),
  event(6, { type: "run_finished", outcome: "passed", reason: "every test passed" }),
];

export const INVESTIGATION = "run-7a1e2c3d4b5f4a6e8d9c0b1a2f3e4d5c";
export const COMMIT = "0123456789abcdef0123456789abcdef01234567";

/** The concluded investigation of unit 1.2 of the failed run. */
export const investigation: Investigation = {
  id: INVESTIGATION,
  parent_run: RUN,
  unit: "1.2",
  lifecycle: "finished",
  outcome: "concluded",
  reason: "the investigation ended concluded",
  repositories: [{ name: "orders", url: "https://github.com/example/orders.git", ref: "v2.4.1" }],
  created_at: "2026-10-17T10:05:00Z",
  finished_at: "2026-10-17T10:09:00Z",
  result: {
    outcome: "concluded",
    summary: "The orders handler returns 500 when the database pool is empty.",
    sources: [{ name: "orders", commit: COMMIT }],
    hypotheses: [
      {
        statement: "`list_orders` unwraps the pool result instead of waiting.",
        confidence: "high",
        evidence: ["ev-2"],
        code: [{ repository: "orders", commit: COMMIT, path: "src/orders.rs", start_line: 40, end_line: 52 }],
      },
    ],
    bug: { url: "https://github.com/example/orders/issues/7", title: "Orders list fails while the pool starts" },
  },
};

const agentShell = (id: string, command: string, text: string): Evidence => ({
  id,
  unit: "investigation",
  attempt: 1,
  recorded_at: "2026-10-17T10:06:00Z",
  duration_ms: 50,
  tool: "shell",
  command,
  stdout: { text, total_bytes: text.length },
  stderr: { text: "", total_bytes: 0 },
  exit_code: 0,
  termination: "exited",
});

/** The report of the run that executed the investigation: its own evidence. */
export const investigationReport: RunReport = {
  ...report,
  run_id: INVESTIGATION,
  environment: "investigator",
  counts: { ...counts, passed: 1 },
  outcome: "passed",
  units: [],
  cleanup_not_run: [],
  investigation: investigation.result,
  evidence: [
    agentShell("ev-1", "git clone --depth 1 ...", COMMIT),
    agentShell("ev-2", "ripwire . --for='List orders'", "src/orders.rs:40 list_orders"),
  ],
};
