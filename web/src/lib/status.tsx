// How every lifecycle, outcome, action and criterion state looks: a tone,
// an icon and a text label, never color alone (artwork/BRAND.md section 3).
import {
  Ban,
  CircleCheck,
  CircleHelp,
  CircleMinus,
  CircleSlash,
  CircleX,
  Clock,
  Hand,
  LoaderCircle,
  type LucideIcon,
  OctagonAlert,
  Send,
  TriangleAlert,
  Undo2,
} from "lucide-react";

import type { ActionState, InvestigationOutcome, Lifecycle, Outcome } from "../api/types";

export type Tone = "pass" | "fail" | "warn" | "run" | "neutral";

export interface StatusLook {
  label: string;
  tone: Tone;
  icon: LucideIcon;
  /** Animated while work is in progress. */
  live?: boolean;
}

export const OUTCOME: Record<Outcome, StatusLook> = {
  passed: { label: "Passed", tone: "pass", icon: CircleCheck },
  failed: { label: "Failed", tone: "fail", icon: CircleX },
  error: { label: "Execution error", tone: "fail", icon: OctagonAlert },
  blocked: { label: "Blocked", tone: "warn", icon: Ban },
  inconclusive: { label: "Inconclusive", tone: "warn", icon: CircleHelp },
  skipped: { label: "Skipped", tone: "neutral", icon: CircleMinus },
  cancelled: { label: "Cancelled", tone: "neutral", icon: CircleSlash },
};

export const LIFECYCLE: Record<Lifecycle, StatusLook> = {
  accepted: { label: "Accepted", tone: "neutral", icon: Clock },
  starting: { label: "Starting", tone: "run", icon: LoaderCircle, live: true },
  running: { label: "Running", tone: "run", icon: LoaderCircle, live: true },
  stopping: { label: "Stopping", tone: "warn", icon: LoaderCircle, live: true },
  finished: { label: "Finished", tone: "neutral", icon: CircleCheck },
};

export const ACTION: Record<ActionState, StatusLook> = {
  pending_approval: { label: "Pending approval", tone: "warn", icon: Hand },
  queued: { label: "Queued", tone: "neutral", icon: Clock },
  executing: { label: "Executing", tone: "run", icon: Send, live: true },
  rejected: { label: "Rejected", tone: "neutral", icon: Undo2 },
  completed: { label: "Completed", tone: "pass", icon: CircleCheck },
  error: { label: "Delivery error", tone: "fail", icon: TriangleAlert },
  unknown: { label: "Delivery unknown", tone: "warn", icon: CircleHelp },
};

export const CRITERION: Record<"satisfied" | "contradicted" | "undetermined", StatusLook> = {
  satisfied: { label: "Satisfied", tone: "pass", icon: CircleCheck },
  contradicted: { label: "Contradicted", tone: "fail", icon: CircleX },
  undetermined: { label: "Undetermined", tone: "warn", icon: CircleHelp },
};

export const BASIS: Record<"mechanical" | "model_judgment", string> = {
  mechanical: "Mechanical check",
  model_judgment: "Model judgment",
};

/** A unit that has not finished: running when its last attempt is open. */
export const PENDING: StatusLook = { label: "Not started", tone: "neutral", icon: Clock };
export const IN_PROGRESS: StatusLook = {
  label: "In progress",
  tone: "run",
  icon: LoaderCircle,
  live: true,
};

/** The status a run shows: its outcome once finished, else its lifecycle. */
export function runLook(lifecycle: Lifecycle, outcome?: Outcome | null): StatusLook {
  if (lifecycle === "finished" && outcome) return OUTCOME[outcome];
  return LIFECYCLE[lifecycle];
}

export const INVESTIGATION: Record<InvestigationOutcome, StatusLook> = {
  concluded: { label: "Concluded", tone: "pass", icon: CircleCheck },
  inconclusive: { label: "Inconclusive", tone: "warn", icon: CircleHelp },
  error: { label: "Execution error", tone: "fail", icon: OctagonAlert },
  cancelled: { label: "Cancelled", tone: "neutral", icon: CircleSlash },
};

export const CONFIDENCE: Record<"high" | "medium" | "low", string> = {
  high: "High confidence",
  medium: "Medium confidence",
  low: "Low confidence",
};

/** The status an investigation shows: its outcome once finished, else its lifecycle. */
export function investigationLook(lifecycle: Lifecycle, outcome?: InvestigationOutcome | null): StatusLook {
  if (lifecycle === "finished" && outcome) return INVESTIGATION[outcome];
  return LIFECYCLE[lifecycle];
}
