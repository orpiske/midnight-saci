// Starts a new run of a finished run's plan: the retained bundle, with the
// same environment and policy. It uses the retry-safe submission of the
// New run page, so a lost answer never starts two runs.
import { RotateCcw } from "lucide-react";
import { useRef, useState } from "react";

import type { RunPlan, RunSummary, SubmitRun } from "../../api/types";
import { Dialog } from "../../components/Dialog";
import { Callout, describeError } from "../../components/Feedback";
import { useAnnounce } from "../../lib/announce";
import { navigate } from "../../lib/router";
import { useClient } from "../connection/connection";
import { type Pending, type Submission, keyFor, payloadText, send } from "../submission/submit";

/** The submission that runs `plan` again with the settings of `run`. */
export function rerunRequest(run: RunSummary, plan: RunPlan): SubmitRun {
  return {
    bundle: { entry: plan.entry, documents: plan.documents },
    environment: run.environment,
    policy: run.policy,
  };
}

export function RunAgain({ run, plan }: { run: RunSummary; plan: RunPlan }) {
  const client = useClient();
  const announce = useAnnounce();
  const [open, setOpen] = useState(false);
  const [submission, setSubmission] = useState<Submission>({ kind: "idle" });
  const pending = useRef<Pending | null>(null);
  const busy = submission.kind === "sending";

  const start = async () => {
    if (busy) return;
    const request = rerunRequest(run, plan);
    const attempt = keyFor(pending.current, payloadText(request));
    pending.current = attempt;
    setSubmission({ kind: "sending", key: attempt.key });
    try {
      const result = await send(client, request, attempt);
      setSubmission(result);
      if (result.kind === "accepted") {
        pending.current = null;
        announce("Run accepted. Opening it.");
        navigate(`/runs/${encodeURIComponent(result.run.id)}`);
      } else if (result.kind !== "unresolved") {
        // A definite refusal: nothing was stored, so a new try gets a new key.
        pending.current = null;
      }
    } catch (error) {
      setSubmission({ kind: "unresolved", key: attempt.key, reason: describeError(error) });
    }
  };

  const close = () => {
    if (!busy) setOpen(false);
  };

  return (
    <>
      <button type="button" className="btn" onClick={() => setOpen(true)}>
        <RotateCcw size={16} aria-hidden="true" />
        Run again
      </button>
      <Dialog
        open={open}
        title="Run this plan again?"
        onClose={close}
        footer={
          <>
            <button type="button" className="btn" onClick={close} disabled={busy}>
              Close
            </button>
            <button type="button" className="btn btn-primary" onClick={() => void start()} disabled={busy}>
              <RotateCcw size={16} aria-hidden="true" />
              {busy ? "Starting…" : submission.kind === "unresolved" ? "Retry start" : "Start run"}
            </button>
          </>
        }
      >
        <p>
          Saci starts a new run of <strong>{plan.title}</strong> with the documents this run
          executed, in environment <strong>{run.environment}</strong> with policy{" "}
          <strong>{run.policy}</strong>. The environment and policy use the backend's current
          configuration. This run does not change.
        </p>
        {submission.kind === "invalid" ? (
          <Callout tone="fail" title="The backend did not accept the run" role="alert">
            <ul className="stack-sm" style={{ margin: 0, paddingLeft: 18 }}>
              {(submission.details.length > 0 ? submission.details : [submission.message]).map((text, i) => (
                <li key={i}>{text}</li>
              ))}
            </ul>
          </Callout>
        ) : null}
        {submission.kind === "refused" ? (
          <Callout tone="fail" title="The backend refused the submission" role="alert">
            {describeError(submission.error)}
          </Callout>
        ) : null}
        {submission.kind === "unresolved" ? (
          <Callout tone="warn" title="The backend did not confirm the submission" role="alert">
            {submission.reason} A run may already exist. Retrying sends the same request with the
            same key, so the backend creates at most one run.
          </Callout>
        ) : null}
      </Dialog>
    </>
  );
}
