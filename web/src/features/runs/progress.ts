import type { OutcomeCounts } from "../../api/types";

/** Units that have an outcome. */
export function completed(counts: OutcomeCounts): number {
  return (
    counts.passed +
    counts.failed +
    counts.blocked +
    counts.inconclusive +
    counts.error +
    counts.skipped +
    counts.cancelled
  );
}
