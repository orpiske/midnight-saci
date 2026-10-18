// Retry-safe submission (docs/api.md "Retries and idempotency"). One
// idempotency key follows one exact payload until the backend answers
// definitively; a lost answer never starts a fresh run by itself, and
// retrying it reuses the key, so at most one run is created.
import { ApiError, type Client, NetworkError } from "../../api/client";
import type { PlanDiagnostic, RunSummary, SubmitRun } from "../../api/types";
import { payloadText } from "./bundle";

export type Submission =
  | { kind: "idle" }
  | { kind: "sending"; key: string }
  | { kind: "accepted"; run: RunSummary; replayed: boolean }
  /** 422: the plan, environment or policy is invalid; nothing was stored. */
  | { kind: "invalid"; message: string; details: string[]; diagnostics: PlanDiagnostic[] }
  /** Another definite refusal (400, 401, 409): nothing was stored. */
  | { kind: "refused"; error: ApiError }
  /** No definite answer: the run may or may not exist. */
  | { kind: "unresolved"; key: string; reason: string };

export interface Pending {
  payload: string;
  key: string;
}

export function newKey(): string {
  return `saci-web-${crypto.randomUUID()}`;
}

/** The key for `payload`: the pending one when the payload is unchanged. */
export function keyFor(pending: Pending | null, payload: string, make = newKey): Pending {
  return pending && pending.payload === payload ? pending : { payload, key: make() };
}

/** Whether an error leaves the outcome of a mutation unknown. */
export function ambiguous(error: unknown): boolean {
  return error instanceof NetworkError || (error instanceof ApiError && error.status >= 500);
}

/** Sends `request` once with `pending.key` and classifies the answer. */
export async function send(
  client: Client,
  request: SubmitRun,
  pending: Pending,
): Promise<Submission> {
  try {
    const answer = await client.post<RunSummary>("/runs", request, {
      headers: { "Idempotency-Key": pending.key },
    });
    return { kind: "accepted", run: answer.body, replayed: answer.status === 200 };
  } catch (error) {
    if (ambiguous(error)) {
      return {
        kind: "unresolved",
        key: pending.key,
        reason: error instanceof ApiError ? error.message : "The connection failed before the backend answered.",
      };
    }
    if (error instanceof ApiError && error.status === 422) {
      return {
        kind: "invalid",
        message: error.message,
        details: error.details,
        diagnostics: error.diagnostics,
      };
    }
    if (error instanceof ApiError) return { kind: "refused", error };
    throw error;
  }
}

export { payloadText };
