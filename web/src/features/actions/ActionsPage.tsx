// Outcome actions across runs, one state at a time, oldest first, the way
// the backend pages them (docs/api.md "Outcome actions").
import { RefreshCw } from "lucide-react";
import { useState } from "react";

import { query } from "../../api/client";
import { ACTION_STATES, type Action, type ActionState, type Page } from "../../api/types";
import { EmptyState, ErrorNotice, Loading } from "../../components/Feedback";
import { useResource } from "../../lib/resource";
import { setSearch, useLocation } from "../../lib/router";
import { ACTION } from "../../lib/status";
import { useClient } from "../connection/connection";
import { ActionDialog } from "./ActionDialog";
import { ActionsTable } from "./ActionsTable";

const PAGE = 50;

const EMPTY: Record<ActionState, string> = {
  pending_approval: "Nothing waits for approval.",
  queued: "Nothing is queued.",
  executing: "Nothing is being sent.",
  rejected: "No action was rejected.",
  completed: "No action completed yet.",
  error: "No delivery failed.",
  unknown: "No delivery is uncertain.",
};

function isState(value: string | null): value is ActionState {
  return ACTION_STATES.some((state) => state === value);
}

export function ActionsPage() {
  const client = useClient();
  const { search } = useLocation();
  const wanted = search.get("state");
  const state: ActionState = isState(wanted) ? wanted : "pending_approval";
  const first = useResource(
    (signal) => client.get<Page<Action>>(`/actions${query({ state, limit: PAGE })}`, { signal }),
    [client, state],
  );
  const [more, setMore] = useState<{ state: ActionState; items: Action[]; cursor?: string } | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [moreError, setMoreError] = useState<unknown>(null);

  const extra = more?.state === state ? more : null;
  const rows = [...(first.resource.data?.items ?? []), ...(extra?.items ?? [])];
  const cursor = extra ? extra.cursor : first.resource.data?.next_cursor;
  const runId = search.get("run");
  const name = search.get("action");

  const reload = () => {
    setMore(null);
    setMoreError(null);
    void first.reload();
  };
  const loadMore = async () => {
    if (!cursor) return;
    setLoadingMore(true);
    setMoreError(null);
    try {
      const page = await client.get<Page<Action>>(`/actions${query({ state, limit: PAGE, cursor })}`);
      setMore({ state, items: [...(extra?.items ?? []), ...page.items], cursor: page.next_cursor });
    } catch (error) {
      setMoreError(error);
    } finally {
      setLoadingMore(false);
    }
  };

  return (
    <div className="page">
      <div className="page-header">
        <div className="stack-sm">
          <h1>Outcome actions</h1>
          <p className="muted">What runs asked to send when they finished, by delivery state.</p>
        </div>
        <div className="actions">
          <button type="button" className="btn" onClick={reload}>
            <RefreshCw size={16} aria-hidden="true" />
            Refresh
          </button>
        </div>
      </div>
      <div className="segmented" role="group" aria-label="Delivery state">
        {ACTION_STATES.map((option) => (
          <button
            key={option}
            type="button"
            aria-pressed={option === state}
            onClick={() => setSearch({ state: option === "pending_approval" ? null : option, run: null, action: null })}
          >
            {ACTION[option].label}
          </button>
        ))}
      </div>
      <div className="stack" style={{ marginTop: 16 }}>
        {first.resource.state === "loading" ? <Loading label="Loading actions…" /> : null}
        {first.resource.state === "error" ? (
          <ErrorNotice error={first.resource.error} title="Actions could not be loaded" retry={reload} />
        ) : null}
        {first.resource.data && rows.length === 0 ? <EmptyState title={EMPTY[state]} /> : null}
        {rows.length > 0 ? (
          <>
            <ActionsTable
              actions={rows}
              caption={`${ACTION[state].label} actions, oldest first`}
              showRun
              open={(action) => setSearch({ run: action.run_id, action: action.name }, true)}
            />
            <div className="table-footer">
              {moreError ? (
                <ErrorNotice error={moreError} title="More actions could not be loaded" retry={() => void loadMore()} />
              ) : cursor ? (
                <button type="button" className="btn" onClick={() => void loadMore()} disabled={loadingMore}>
                  {loadingMore ? "Loading…" : "Load more"}
                </button>
              ) : (
                <span className="muted small">{rows.length === 1 ? "1 action" : `All ${rows.length} actions`}</span>
              )}
            </div>
          </>
        ) : null}
      </div>
      {runId && name ? (
        <ActionDialog
          run={runId}
          name={name}
          onClose={() => setSearch({ run: null, action: null })}
          onChanged={reload}
        />
      ) : null}
    </div>
  );
}
