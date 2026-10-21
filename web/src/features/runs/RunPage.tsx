import { CircleSlash, Download, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useId, useRef, useState } from "react";

import { ApiError, query, seg } from "../../api/client";
import type { EventPage, RunEvent, RunPlan, RunReport, RunSummary, WorkerImage } from "../../api/types";
import { CopyText } from "../../components/Copy";
import { Dialog } from "../../components/Dialog";
import { Callout, ErrorNotice, Loading, describeError } from "../../components/Feedback";
import { Badge } from "../../components/Status";
import { Tabs, tabPanelProps } from "../../components/Tabs";
import { RunActionsTab } from "../actions/RunActionsTab";
import { useAnnounce } from "../../lib/announce";
import { between, formatTime, plural } from "../../lib/format";
import { useResource } from "../../lib/resource";
import { Link, setSearch, useLocation } from "../../lib/router";
import { OUTCOME, runLook } from "../../lib/status";
import { useClient } from "../connection/connection";
import { ActivityTab } from "./ActivityTab";
import { EnvironmentTab } from "./EnvironmentTab";
import { type FollowState, follow } from "./follow";
import { PlanTab } from "./PlanTab";
import { completed } from "./progress";
import { ResultsTab } from "./ResultsTab";

type TabKey = "results" | "activity" | "plan" | "environment" | "actions";

const TABS: readonly { key: TabKey; label: string }[] = [
  { key: "results", label: "Results" },
  { key: "activity", label: "Activity" },
  { key: "plan", label: "Plan" },
  { key: "environment", label: "Environment" },
  { key: "actions", label: "Outcome actions" },
];

/** Events kept in memory for the activity view; older ones are dropped. */
export const KEPT_EVENTS = 2000;
/** How often the summary and report refresh while events arrive. */
const REFRESH_MS = 2000;

function isTab(value: string | null): value is TabKey {
  return TABS.some((tab) => tab.key === value);
}

export interface Activity {
  events: RunEvent[];
  /** Events received since the page opened, including dropped ones. */
  total: number;
  state: FollowState;
}

/** Follows the run's events, refreshing the summary and report at most
 * every two seconds while they arrive and once more when the run ends. */
function useActivity(id: string, refresh: () => void, finished: () => void): Activity {
  const client = useClient();
  const [activity, setActivity] = useState<Activity>({
    events: [],
    total: 0,
    state: { status: "live", lastUpdate: null },
  });
  const saved = useRef({ refresh, finished });
  saved.current = { refresh, finished };

  useEffect(() => {
    const controller = new AbortController();
    let timer: number | null = null;
    const schedule = () => {
      if (timer !== null) return;
      timer = window.setTimeout(() => {
        timer = null;
        saved.current.refresh();
      }, REFRESH_MS);
    };
    void follow(
      {
        fetchPage: (after, wait, signal) =>
          client.get<EventPage>(`/runs/${seg(id)}/events${query({ after, wait, limit: 200 })}`, {
            signal,
          }),
        onEvents: (events) => {
          setActivity((previous) => ({
            ...previous,
            events: [...previous.events, ...events].slice(-KEPT_EVENTS),
            total: previous.total + events.length,
          }));
          schedule();
        },
        onState: (state) => setActivity((previous) => ({ ...previous, state })),
        onFinished: () => {
          if (timer !== null) window.clearTimeout(timer);
          timer = null;
          saved.current.finished();
        },
      },
      controller.signal,
    );
    return () => {
      controller.abort();
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [client, id]);
  return activity;
}

export function RunPage({ id }: { id: string }) {
  const client = useClient();
  const announce = useAnnounce();
  const { search } = useLocation();
  const requested = search.get("tab");
  const tab: TabKey = isTab(requested) ? requested : "results";
  const tabsId = useId();

  const summary = useResource(
    (signal) => client.get<RunSummary>(`/runs/${seg(id)}`, { signal }),
    [client, id],
  );
  const report = useResource(
    (signal) => client.get<RunReport>(`/runs/${seg(id)}/report`, { signal }),
    [client, id],
  );
  const plan = useResource(
    (signal) => client.get<RunPlan>(`/runs/${seg(id)}/plan`, { signal }),
    [client, id],
  );

  const latest = useRef<RunSummary | undefined>(undefined);
  latest.current = summary.resource.data;
  const refresh = useCallback(() => {
    void summary.reload();
    void report.reload();
  }, [summary.reload, report.reload]);
  const finished = useCallback(() => {
    const before = latest.current;
    void report.reload();
    void summary.reload().then(() => {
      const after = latest.current;
      if (before && before.lifecycle !== "finished" && after?.outcome) {
        announce(`The run finished: ${OUTCOME[after.outcome].label}.`);
      }
    });
  }, [summary.reload, report.reload, announce]);
  const activity = useActivity(id, refresh, finished);

  const run = summary.resource.data;
  if (summary.resource.state === "loading") return <div className="page"><Loading label="Loading the run…" /></div>;
  if (!run) {
    const missing = summary.resource.error instanceof ApiError && summary.resource.error.status === 404;
    return (
      <div className="page stack">
        <Breadcrumb />
        <ErrorNotice
          error={summary.resource.error}
          title={missing ? "No such run" : "The run could not be loaded"}
          retry={missing ? undefined : () => void summary.reload()}
        />
      </div>
    );
  }

  return (
    <div className="page">
      <Breadcrumb />
      <RunHeader run={run} report={report.resource.data} onChanged={summary.set} reload={refresh} />
      {activity.state.status === "reconnecting" ? (
        <Callout tone="warn" title="Reconnecting" role="status">
          Lost contact with the backend; live progress resumes from the last event once it answers.
        </Callout>
      ) : null}
      <Tabs
        id={tabsId}
        label="Run views"
        items={TABS}
        selected={tab}
        onSelect={(key) => setSearch({ tab: key === "results" ? null : key })}
      />
      <div {...tabPanelProps(tabsId, tab)} className="tab-panel">
        {tab === "results" ? (
          <ResultsTab report={report} plan={plan.resource.data} search={search} />
        ) : tab === "activity" ? (
          <ActivityTab activity={activity} />
        ) : tab === "plan" ? (
          <PlanTab plan={plan} search={search} />
        ) : tab === "environment" ? (
          <EnvironmentTab report={report} />
        ) : (
          <RunActionsTab run={run} />
        )}
      </div>
    </div>
  );
}

function Breadcrumb() {
  return (
    <nav className="breadcrumb" aria-label="Breadcrumb">
      <Link to="/runs">Runs</Link>
      <span aria-hidden="true">/</span>
      <span aria-current="page">Run</span>
    </nav>
  );
}

function RunHeader({
  run,
  report,
  onChanged,
  reload,
}: {
  run: RunSummary;
  report: RunReport | undefined;
  onChanged: (run: RunSummary) => void;
  reload: () => void;
}) {
  const done = completed(run.counts);
  const share = run.total_units > 0 ? Math.round((done / run.total_units) * 100) : 0;
  const active = run.lifecycle !== "finished";
  return (
    <header className="run-header">
      <div className="page-header">
        <div className="stack-sm">
          <h1>{run.plan_title}</h1>
          <div className="row">
            <Badge look={runLook(run.lifecycle, run.outcome)} />
            {run.lifecycle === "finished" ? null : <span className="tag">{plural(done, "unit")} of {run.total_units} done</span>}
            {run.cancel_requested_at && active ? <span className="tag">Cancellation requested</span> : null}
          </div>
        </div>
        <div className="actions">
          <button type="button" className="btn" onClick={reload}>
            <RefreshCw size={16} aria-hidden="true" />
            Refresh
          </button>
          <DownloadReport run={run} report={report} />
          {active ? <CancelRun run={run} onChanged={onChanged} /> : null}
        </div>
      </div>
      {run.reason ? (
        <p>
          <span className="muted">Why: </span>
          {run.reason}
        </p>
      ) : null}
      <div className="row">
        <div
          className="progress"
          role="progressbar"
          aria-label="Units with an outcome"
          aria-valuemin={0}
          aria-valuemax={run.total_units}
          aria-valuenow={done}
        >
          <span style={{ width: `${share}%` }} />
        </div>
        <span className="muted small num">
          {done} / {run.total_units} units
          {run.current_unit && active ? ` · now ${run.current_unit}` : ""}
        </span>
      </div>
      <div className="meta">
        <span>
          Run <CopyText value={run.id} title="run ID" />
        </span>
        <span>Environment {run.environment}</span>
        <span>Policy {run.policy}</span>
        {report?.model ? <span>Model {report.model}</span> : null}
        {report?.worker_image ? <ImageMeta image={report.worker_image} /> : null}
        <span className="num">Created {formatTime(run.created_at)}</span>
        {report?.started_at ? <span className="num">Started {formatTime(report.started_at)}</span> : null}
        {run.finished_at ? (
          <span className="num">
            Finished {formatTime(run.finished_at)} ({between(report?.started_at ?? run.created_at, run.finished_at)})
          </span>
        ) : null}
      </div>
    </header>
  );
}

/** The worker's image, with the digest (or, for a local build, the image ID) that names the exact build. */
function ImageMeta({ image }: { image: WorkerImage }) {
  const exact = image.digest ?? image.id;
  const kind = image.digest ? "Digest" : "Image ID";
  return (
    <>
      <span>
        Image <CopyText value={image.reference} title="image reference" />
      </span>
      {exact ? (
        <span>
          {kind} <CopyText value={exact} label={shortDigest(exact)} title={kind.toLowerCase()} />
        </span>
      ) : null}
    </>
  );
}

/** `sha256:4b2f8e0c1d3a…` for a full digest. */
function shortDigest(digest: string): string {
  const at = digest.indexOf(":") + 1;
  return digest.length > at + 13 ? `${digest.slice(0, at + 12)}…` : digest;
}

function DownloadReport({ run, report }: { run: RunSummary; report: RunReport | undefined }) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!report) return;
    const blob = new Blob([`${JSON.stringify(report, null, 2)}\n`], { type: "application/json" });
    const next = URL.createObjectURL(blob);
    setUrl(next);
    return () => URL.revokeObjectURL(next);
  }, [report]);
  if (!report || !url) return null;
  const final = report.lifecycle === "finished";
  return (
    <a className="btn" href={url} download={`${run.id}-report.json`}>
      <Download size={16} aria-hidden="true" />
      {final ? "Download report" : "Download report so far"}
    </a>
  );
}

function CancelRun({ run, onChanged }: { run: RunSummary; onChanged: (run: RunSummary) => void }) {
  const client = useClient();
  const announce = useAnnounce();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requested = Boolean(run.cancel_requested_at);

  const cancel = async () => {
    setBusy(true);
    setError(null);
    try {
      const answer = await client.post<RunSummary>(`/runs/${seg(run.id)}/cancellation`, {});
      onChanged(answer.body);
      setOpen(false);
      announce("Cancellation requested. The run stops after the current operation.");
    } catch (failure) {
      if (failure instanceof ApiError && failure.code === "run_finished") {
        setOpen(false);
        const fresh = await client.get<RunSummary>(`/runs/${seg(run.id)}`).catch(() => null);
        if (fresh) onChanged(fresh);
        announce("The run had already finished.");
      } else {
        setError(describeError(failure));
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <button
        type="button"
        className="btn btn-danger"
        onClick={() => setOpen(true)}
        disabled={requested}
        title={requested ? "Cancellation was already requested" : undefined}
      >
        <CircleSlash size={16} aria-hidden="true" />
        {requested ? "Cancellation requested" : "Cancel run"}
      </button>
      <Dialog
        open={open}
        title="Cancel this run?"
        onClose={() => setOpen(false)}
        footer={
          <>
            <button type="button" className="btn" onClick={() => setOpen(false)}>
              Keep running
            </button>
            <button type="button" className="btn btn-danger" onClick={() => void cancel()} disabled={busy}>
              {busy ? "Requesting…" : "Request cancellation"}
            </button>
          </>
        }
      >
        <p>
          Saci stops the current tool call and skips the remaining units, including cleanup steps;
          the report lists the cleanup that did not run. The outcome becomes Cancelled unless the
          run ends for another reason first.
        </p>
        {error ? (
          <Callout tone="fail" title="Cancellation was not recorded" role="alert">
            {error}
          </Callout>
        ) : null}
      </Dialog>
    </>
  );
}
