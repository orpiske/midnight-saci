import { Plus, RefreshCw } from "lucide-react";
import { useCallback, useState } from "react";

import { query, seg } from "../../api/client";
import type { Page, RunSummary } from "../../api/types";
import { CopyText } from "../../components/Copy";
import { EmptyState, ErrorNotice, Loading } from "../../components/Feedback";
import { Badge } from "../../components/Status";
import { formatRelative, formatTime, shortId } from "../../lib/format";
import { useInterval, useResource } from "../../lib/resource";
import { Link } from "../../lib/router";
import { runLook } from "../../lib/status";
import { useClient } from "../connection/connection";
import { completed } from "./progress";

const PAGE = 50;

export function RunsPage() {
  const client = useClient();
  const first = useResource(
    (signal) => client.get<Page<RunSummary>>(`/runs${query({ limit: PAGE })}`, { signal }),
    [client],
  );
  const [more, setMore] = useState<RunSummary[]>([]);
  const [cursor, setCursor] = useState<string | undefined | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [moreError, setMoreError] = useState<unknown>(null);
  const [fresh, setFresh] = useState<Record<string, RunSummary>>({});

  const firstPage = first.resource.data;
  const nextCursor = cursor === null ? firstPage?.next_cursor : cursor;
  const rows = [...(firstPage?.items ?? []), ...more].map((run) => fresh[run.id] ?? run);

  const loadMore = async () => {
    if (!nextCursor) return;
    setLoadingMore(true);
    setMoreError(null);
    try {
      const page = await client.get<Page<RunSummary>>(
        `/runs${query({ limit: PAGE, cursor: nextCursor })}`,
      );
      setMore((previous) => [...previous, ...page.items]);
      setCursor(page.next_cursor);
    } catch (error) {
      setMoreError(error);
    } finally {
      setLoadingMore(false);
    }
  };

  const active = rows.filter((run) => run.lifecycle !== "finished");
  const refreshActive = useCallback(async () => {
    const updates = await Promise.allSettled(
      active.map((run) => client.get<RunSummary>(`/runs/${seg(run.id)}`)),
    );
    const next: Record<string, RunSummary> = {};
    for (const update of updates) {
      if (update.status === "fulfilled") next[update.value.id] = update.value;
    }
    setFresh((previous) => ({ ...previous, ...next }));
  }, [active, client]);
  useInterval(() => void refreshActive(), 5000, active.length > 0);

  const reloadAll = () => {
    setMore([]);
    setCursor(null);
    setFresh({});
    void first.reload();
  };

  return (
    <div className="page">
      <div className="page-header">
        <div className="stack-sm">
          <h1>Runs</h1>
          <p className="muted">Newest first. Active runs refresh on their own.</p>
        </div>
        <div className="actions">
          <button type="button" className="btn" onClick={reloadAll}>
            <RefreshCw size={16} aria-hidden="true" />
            Refresh
          </button>
          <Link to="/runs/new" className="btn btn-primary">
            <Plus size={16} aria-hidden="true" />
            New run
          </Link>
        </div>
      </div>
      {first.resource.state === "loading" ? <Loading label="Loading runs…" /> : null}
      {first.resource.state === "error" ? (
        <ErrorNotice error={first.resource.error} title="Runs could not be loaded" retry={reloadAll} />
      ) : null}
      {firstPage && rows.length === 0 ? (
        <EmptyState
          title="Nothing broken yet. The Saci is still looking."
          action={
            <Link to="/runs/new" className="btn btn-primary">
              <Plus size={16} aria-hidden="true" />
              New run
            </Link>
          }
        >
          No runs on this backend yet. Start one from a Markdown test plan.
        </EmptyState>
      ) : null}
      {rows.length > 0 ? (
        <>
          <RunsTable rows={rows} />
          <div className="table-footer">
            {moreError ? (
              <ErrorNotice error={moreError} title="More runs could not be loaded" retry={() => void loadMore()} />
            ) : nextCursor ? (
              <button type="button" className="btn" onClick={() => void loadMore()} disabled={loadingMore}>
                {loadingMore ? "Loading…" : "Load more"}
              </button>
            ) : (
              <span className="muted small">
                {rows.length === 1 ? "1 run" : `All ${rows.length} runs`}
              </span>
            )}
          </div>
        </>
      ) : null}
    </div>
  );
}

function RunsTable({ rows }: { rows: RunSummary[] }) {
  return (
    <div className="table-wrap">
      <table className="table">
        <caption className="visually-hidden">Runs, newest first</caption>
        <thead>
          <tr>
            <th scope="col">Plan</th>
            <th scope="col">Status</th>
            <th scope="col">Units</th>
            <th scope="col">Environment</th>
            <th scope="col">Policy</th>
            <th scope="col">Created</th>
            <th scope="col">Run ID</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((run) => (
            <tr key={run.id}>
              <td className="title-cell">
                <Link to={`/runs/${seg(run.id)}`}>{run.plan_title}</Link>
              </td>
              <td>
                <span className="row">
                  <Badge look={runLook(run.lifecycle, run.outcome)} />
                  {run.cancel_requested_at && run.lifecycle !== "finished" ? (
                    <span className="tag">Cancellation requested</span>
                  ) : null}
                </span>
              </td>
              <td className="num">
                {completed(run.counts)} / {run.total_units}
              </td>
              <td>{run.environment}</td>
              <td>{run.policy}</td>
              <td className="num" title={formatTime(run.created_at)}>
                {formatRelative(run.created_at)}
              </td>
              <td>
                <CopyText value={run.id} label={shortId(run.id)} title="run ID" />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
