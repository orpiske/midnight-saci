// The investigations of all runs, newest first, with the cursor of the
// API. Active investigations refresh on their own.
import { RefreshCw } from "lucide-react";
import { useCallback, useState } from "react";

import { query, seg } from "../../api/client";
import type { Investigation, Page } from "../../api/types";
import { CopyText } from "../../components/Copy";
import { EmptyState, ErrorNotice, Loading } from "../../components/Feedback";
import { Badge } from "../../components/Status";
import { formatRelative, formatTime, plural, shortId } from "../../lib/format";
import { useInterval, useResource } from "../../lib/resource";
import { Link } from "../../lib/router";
import { investigationLook } from "../../lib/status";
import { useClient } from "../connection/connection";

const PAGE = 50;

export function InvestigationsPage() {
  const client = useClient();
  const first = useResource(
    (signal) => client.get<Page<Investigation>>(`/investigations${query({ limit: PAGE })}`, { signal }),
    [client],
  );
  const [more, setMore] = useState<Investigation[]>([]);
  const [cursor, setCursor] = useState<string | undefined | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [moreError, setMoreError] = useState<unknown>(null);
  const [fresh, setFresh] = useState<Record<string, Investigation>>({});

  const firstPage = first.resource.data;
  const nextCursor = cursor === null ? firstPage?.next_cursor : cursor;
  const rows = [...(firstPage?.items ?? []), ...more].map((item) => fresh[item.id] ?? item);

  const loadMore = async () => {
    if (!nextCursor) return;
    setLoadingMore(true);
    setMoreError(null);
    try {
      const page = await client.get<Page<Investigation>>(
        `/investigations${query({ limit: PAGE, cursor: nextCursor })}`,
      );
      setMore((previous) => [...previous, ...page.items]);
      setCursor(page.next_cursor);
    } catch (error) {
      setMoreError(error);
    } finally {
      setLoadingMore(false);
    }
  };

  const active = rows.filter((item) => item.lifecycle !== "finished");
  const refreshActive = useCallback(async () => {
    const updates = await Promise.allSettled(
      active.map((item) => client.get<Investigation>(`/investigations/${seg(item.id)}`)),
    );
    const next: Record<string, Investigation> = {};
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
          <h1>Investigations</h1>
          <p className="muted">
            Probable causes of failed units, found in the source code. Newest first.
          </p>
        </div>
        <div className="actions">
          <button type="button" className="btn" onClick={reloadAll}>
            <RefreshCw size={16} aria-hidden="true" />
            Refresh
          </button>
        </div>
      </div>
      {first.resource.state === "loading" ? <Loading label="Loading investigations…" /> : null}
      {first.resource.state === "error" ? (
        <ErrorNotice error={first.resource.error} title="Investigations could not be loaded" retry={reloadAll} />
      ) : null}
      {firstPage && rows.length === 0 ? (
        <EmptyState title="No investigations yet">
          Start one for a failed unit with <code>saci investigate &lt;run-id&gt;</code> or the
          API (see the investigations documentation).
        </EmptyState>
      ) : null}
      {rows.length > 0 ? (
        <>
          <InvestigationsTable rows={rows} />
          <div className="table-footer">
            {moreError ? (
              <ErrorNotice error={moreError} title="More investigations could not be loaded" retry={() => void loadMore()} />
            ) : nextCursor ? (
              <button type="button" className="btn" onClick={() => void loadMore()} disabled={loadingMore}>
                {loadingMore ? "Loading…" : "Load more"}
              </button>
            ) : (
              <span className="muted small">
                {rows.length === 1 ? "1 investigation" : `All ${rows.length} investigations`}
              </span>
            )}
          </div>
        </>
      ) : null}
    </div>
  );
}

function InvestigationsTable({ rows }: { rows: Investigation[] }) {
  return (
    <div className="table-wrap">
      <table className="table">
        <caption className="visually-hidden">Investigations, newest first</caption>
        <thead>
          <tr>
            <th scope="col">Failed unit</th>
            <th scope="col">Plan</th>
            <th scope="col">Status</th>
            <th scope="col">Hypotheses</th>
            <th scope="col">Created</th>
            <th scope="col">Investigation ID</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((item) => (
            <tr key={item.id}>
              <td className="title-cell">
                <Link to={`/investigations/${seg(item.id)}`}>
                  <span className="mono">{item.unit}</span> {item.unit_title}
                </Link>
              </td>
              <td>
                <Link to={`/runs/${seg(item.parent_run)}?unit=${encodeURIComponent(item.unit)}`}>
                  {item.plan_title}
                </Link>
              </td>
              <td>
                <Badge look={investigationLook(item.lifecycle, item.outcome)} />
              </td>
              <td className="num">
                {item.result ? plural(item.result.hypotheses?.length ?? 0, "hypothesis", "hypotheses") : "None yet"}
              </td>
              <td className="num" title={formatTime(item.created_at)}>
                {formatRelative(item.created_at)}
              </td>
              <td>
                <CopyText value={item.id} label={shortId(item.id)} title="investigation ID" />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
