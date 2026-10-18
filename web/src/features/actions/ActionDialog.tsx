// One outcome action: what it sends, its history, and the decisions a
// person can make about it. Every decision names the digest of the
// arguments shown here, and only the backend's answer changes what is
// displayed; nothing is assumed to have worked.
import { type ReactNode, useState } from "react";

import { ApiError, seg } from "../../api/client";
import type { Action } from "../../api/types";
import { CodeView } from "../../components/Code";
import { CopyText } from "../../components/Copy";
import { Dialog } from "../../components/Dialog";
import { Callout, ErrorNotice, Loading, describeError } from "../../components/Feedback";
import { Badge } from "../../components/Status";
import { useAnnounce } from "../../lib/announce";
import { formatTime } from "../../lib/format";
import { useResource } from "../../lib/resource";
import { Link } from "../../lib/router";
import { ACTION } from "../../lib/status";
import { SafeText } from "../../lib/text";
import { useClient } from "../connection/connection";

const HISTORY_EVENT: Record<NonNullable<Action["history"]>[number]["event"], string> = {
  recorded: "Recorded",
  approved: "Approved",
  rejected: "Rejected",
  claimed: "Claimed for delivery",
  answered: "The tool answered",
  failed: "Delivery failed",
  uncertain: "Delivery uncertain",
  replayed: "Replayed with the same key",
  redispatched: "Sent again",
  reconciled: "Remote result recorded",
};

type Decision = "approval" | "rejection" | "redispatch" | "reconciliation";

const CONFLICTS = new Set([
  "digest_mismatch",
  "action_not_pending",
  "action_not_resolvable",
]);

/** Opens `/runs/{run}/actions/{name}` and lets a person decide on it. */
export function ActionDialog({
  run,
  name,
  onClose,
  onChanged,
}: {
  run: string;
  name: string;
  onClose: () => void;
  onChanged: (action: Action) => void;
}) {
  const client = useClient();
  const path = `/runs/${seg(run)}/actions/${seg(name)}`;
  const action = useResource((signal) => client.get<Action>(path, { signal }), [client, path]);
  const data = action.resource.data;
  return (
    <Dialog
      open
      wide
      onClose={onClose}
      title={
        <span className="stack-sm">
          <span className="mono">{name}</span>
          {data ? <Badge look={ACTION[data.state]} /> : null}
        </span>
      }
    >
      {data ? (
        <ActionBody
          action={data}
          path={path}
          onChanged={(next) => {
            action.set(next);
            onChanged(next);
          }}
          reload={async () => {
            await action.reload();
          }}
        />
      ) : action.resource.state === "error" ? (
        <ErrorNotice error={action.resource.error} title="The action could not be loaded" retry={() => void action.reload()} />
      ) : (
        <Loading label="Loading the action…" />
      )}
    </Dialog>
  );
}

function ActionBody({
  action,
  path,
  onChanged,
  reload,
}: {
  action: Action;
  path: string;
  onChanged: (action: Action) => void;
  reload: () => Promise<void>;
}) {
  return (
    <>
      <dl className="kv">
        <dt>Run</dt>
        <dd>
          <Link to={`/runs/${seg(action.run_id)}?tab=actions`} className="mono">
            {action.run_id}
          </Link>
        </dd>
        <dt>Sends</dt>
        <dd className="mono">
          {action.server} / {action.tool}
        </dd>
        <dt>Approval</dt>
        <dd>{action.approval === "required" ? "Required" : "Not required"}</dd>
        <dt>Attempts</dt>
        <dd className="num">{action.attempts}</dd>
        <dt>Updated</dt>
        <dd className="num">{formatTime(action.updated_at)}</dd>
        {action.idempotency_key ? (
          <>
            <dt>Idempotency key</dt>
            <dd>
              <CopyText value={action.idempotency_key} title="idempotency key" />
            </dd>
          </>
        ) : null}
        <dt>Digest</dt>
        <dd>
          <CopyText value={action.digest} label={`${action.digest.slice(0, 16)}…`} title="arguments digest" />
        </dd>
      </dl>
      <div className="stack-sm">
        <span className="label">Exact arguments</span>
        <CodeView text={JSON.stringify(action.arguments, null, 2)} label="Exact arguments" wrap maxHeight={320} />
      </div>
      {action.decision ? (
        <p className="small">
          {action.decision.verdict === "approved" ? "Approved" : "Rejected"} {formatTime(action.decision.at)}
          {action.decision.note ? (
            <>
              : <SafeText text={action.decision.note} />
            </>
          ) : null}
        </p>
      ) : null}
      {action.error ? (
        <Callout tone={action.state === "unknown" ? "warn" : "fail"} title={action.state === "unknown" ? "Delivery unknown" : "Delivery error"}>
          <SafeText text={action.error} />
        </Callout>
      ) : null}
      {action.result !== undefined ? (
        <div className="stack-sm">
          <span className="label">Result</span>
          <CodeView text={action.result} label="Tool result" wrap maxHeight={240} />
        </div>
      ) : null}
      <Decisions action={action} path={path} onChanged={onChanged} reload={reload} />
      <History action={action} />
    </>
  );
}

function History({ action }: { action: Action }) {
  const history = action.history ?? [];
  if (history.length === 0) return null;
  return (
    <details className="authored">
      <summary>History ({history.length})</summary>
      <ol className="plain-list stack-sm">
        {history.map((entry, index) => (
          <li key={index} className="small">
            <span className="num muted">{formatTime(entry.at)}</span> {HISTORY_EVENT[entry.event]} ·{" "}
            {ACTION[entry.state].label} · attempt {entry.attempt}
            {entry.detail ? (
              <>
                {" "}
                · <SafeText text={entry.detail} />
              </>
            ) : null}
          </li>
        ))}
      </ol>
    </details>
  );
}

function Decisions({
  action,
  path,
  onChanged,
  reload,
}: {
  action: Action;
  path: string;
  onChanged: (action: Action) => void;
  reload: () => Promise<void>;
}) {
  const client = useClient();
  const announce = useAnnounce();
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<{ title: string; text: string } | null>(null);
  const [confirmResend, setConfirmResend] = useState(false);

  const decide = async (decision: Decision, body: Record<string, string>, done: string) => {
    setBusy(true);
    setProblem(null);
    try {
      const answer = await client.post<Action>(`${path}/${decision}`, {
        ...body,
        ...(note.trim() ? { note: note.trim() } : {}),
      });
      onChanged(answer.body);
      setNote("");
      setConfirmResend(false);
      announce(done);
    } catch (error) {
      if (error instanceof ApiError && CONFLICTS.has(error.code)) {
        await reload();
        setProblem({
          title: "The action changed",
          text: `${error.message}. It was reloaded with its current state and arguments; review it again.`,
        });
      } else {
        setProblem({
          title: "The decision may not have been recorded",
          text: `${describeError(error)} Reload the action to see whether it was.`,
        });
      }
    } finally {
      setBusy(false);
    }
  };

  const digest = { digest: action.digest };
  let buttons: ReactNode = null;
  if (action.state === "pending_approval") {
    buttons = (
      <>
        <button type="button" className="btn" disabled={busy} onClick={() => void decide("rejection", digest, "Action rejected.")}>
          Reject
        </button>
        <button
          type="button"
          className="btn btn-primary"
          disabled={busy}
          onClick={() => void decide("approval", digest, "Action approved and queued.")}
        >
          Approve and send
        </button>
      </>
    );
  } else if (action.state === "unknown" || action.state === "error") {
    buttons = (
      <>
        {action.state === "unknown" ? (
          <>
            <button
              type="button"
              className="btn"
              disabled={busy}
              onClick={() => void decide("reconciliation", { state: "completed" }, "Recorded as completed.")}
            >
              It happened
            </button>
            <button
              type="button"
              className="btn"
              disabled={busy}
              onClick={() => void decide("reconciliation", { state: "error" }, "Recorded as not delivered.")}
            >
              It did not happen
            </button>
          </>
        ) : null}
        {confirmResend ? (
          <button
            type="button"
            className="btn btn-danger"
            disabled={busy}
            onClick={() => void decide("redispatch", digest, "Action queued again.")}
          >
            Yes, send again
          </button>
        ) : (
          <button type="button" className="btn" disabled={busy} onClick={() => setConfirmResend(true)}>
            Send again
          </button>
        )}
      </>
    );
  }
  if (!buttons) return null;

  return (
    <section className="stack-sm" aria-label="Decision">
      {action.state === "pending_approval" ? (
        <p className="small">
          Approving sends exactly the arguments above to <span className="mono">{action.tool}</span>.
        </p>
      ) : action.state === "unknown" ? (
        <p className="small">
          The call may have reached the server. Check the remote system, then record what you
          found. Recording sends nothing.
        </p>
      ) : (
        <p className="small">Nothing was sent. You can queue the same arguments again.</p>
      )}
      {confirmResend ? (
        <Callout tone="warn" title="This can act twice">
          {action.idempotency_key
            ? "The call is sent again with the same idempotency key, so a server that honors it does not act twice."
            : "This action has no idempotency key. If the server already acted, it acts again."}
        </Callout>
      ) : null}
      <label className="field">
        <span className="label">Note (kept with the decision)</span>
        <textarea value={note} onChange={(event) => setNote(event.target.value)} rows={2} />
      </label>
      {problem ? (
        <Callout tone="fail" title={problem.title} role="alert">
          {problem.text}
        </Callout>
      ) : null}
      <div className="row" style={{ justifyContent: "flex-end" }}>
        {buttons}
      </div>
    </section>
  );
}
