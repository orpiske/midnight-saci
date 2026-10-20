// Names for the API shapes generated from docs/openapi.yaml into
// schema.d.ts (`npm run api-types`). Edit the OpenAPI file, never these.
import type { components } from "./schema";

type Schemas = components["schemas"];

export type Lifecycle = Schemas["Lifecycle"];
export type Outcome = Schemas["Outcome"];
export type RunSummary = Schemas["RunSummary"];
export type OutcomeCounts = Schemas["OutcomeCounts"];
export type RunReport = Schemas["RunReport"];
export type WorkerImage = Schemas["WorkerImage"];
export type UnitReport = Schemas["UnitReport"];
export type Attempt = Schemas["Attempt"];
export type Assessment = Schemas["Assessment"];
export type CriterionResult = Schemas["CriterionResult"];
export type Criterion = Schemas["Criterion"];
export type ExecutionError = Schemas["ExecutionError"];
export type Evidence = Schemas["Evidence"];
export type CapturedText = Schemas["CapturedText"];
export type RunEvent = Schemas["RunEvent"];
export type EventPage = Schemas["EventPage"];
export type PlanBundle = Schemas["PlanBundle"];
export type SubmitRun = Schemas["SubmitRun"];
export type PlanDiagnostic = Schemas["PlanDiagnostic"];
export type SourceLocation = Schemas["SourceLocation"];
export type ErrorBody = Schemas["ErrorBody"];
export type EnvironmentInfo = Schemas["EnvironmentInfo"];
export type PolicyInfo = Schemas["PolicyInfo"];
export type ActionInfo = Schemas["ActionInfo"];
export type ExecutionLimits = Schemas["ExecutionLimits"];
export type RunPlan = Schemas["RunPlan"];
export type PlannedUnit = Schemas["PlannedUnit"];
export type Action = Schemas["Action"];
export type ActionState = Schemas["ActionState"];
export type ActionHistoryEntry = NonNullable<Action["history"]>[number];

export type ShellEvidence = Extract<Evidence, { tool: "shell" }>;
export type HttpEvidence = Extract<Evidence, { tool: "http" }>;
export type McpEvidence = Extract<Evidence, { tool: "mcp" }>;
export type RejectedEvidence = Extract<Evidence, { tool: "rejected" }>;

export interface Page<T> {
  items: T[];
  next_cursor?: string;
}

export const LIFECYCLES: readonly Lifecycle[] = [
  "accepted",
  "starting",
  "running",
  "stopping",
  "finished",
];

export const OUTCOMES: readonly Outcome[] = [
  "passed",
  "failed",
  "blocked",
  "inconclusive",
  "error",
  "skipped",
  "cancelled",
];

export const ACTION_STATES: readonly ActionState[] = [
  "pending_approval",
  "queued",
  "executing",
  "rejected",
  "completed",
  "error",
  "unknown",
];
