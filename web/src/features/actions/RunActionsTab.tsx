import { seg } from "../../api/client";
import type { Action, RunSummary } from "../../api/types";
import { EmptyState, ErrorNotice, Loading } from "../../components/Feedback";
import { useInterval, useResource } from "../../lib/resource";
import { setSearch, useLocation } from "../../lib/router";
import { useClient } from "../connection/connection";
import { ActionDialog } from "./ActionDialog";
import { ActionsTable } from "./ActionsTable";

const SETTLED = new Set<Action["state"]>(["rejected", "completed", "error", "unknown"]);

/** The outcome actions of one run; `?action=<name>` opens one. */
export function RunActionsTab({ run }: { run: RunSummary }) {
  const client = useClient();
  const { search } = useLocation();
  const list = useResource(
    (signal) => client.get<{ items: Action[] }>(`/runs/${seg(run.id)}/actions`, { signal }),
    [client, run.id, run.lifecycle],
  );
  const actions = list.resource.data?.items;
  const moving = actions?.some((a) => !SETTLED.has(a.state) && a.state !== "pending_approval") ?? false;
  useInterval(() => void list.reload(), 5000, moving);
  const opened = search.get("action");

  if (!actions) {
    if (list.resource.state === "error") {
      return <ErrorNotice error={list.resource.error} title="Actions could not be loaded" retry={() => void list.reload()} />;
    }
    return <Loading label="Loading outcome actions…" />;
  }
  return (
    <>
      {actions.length === 0 ? (
        <EmptyState title="No outcome actions">
          {run.lifecycle === "finished"
            ? `The ${run.policy} policy triggered no action for this outcome.`
            : "Actions are recorded when the run finishes."}
        </EmptyState>
      ) : (
        <ActionsTable
          actions={actions}
          caption="Outcome actions of this run"
          showRun={false}
          open={(action) => setSearch({ action: action.name }, true)}
        />
      )}
      {opened ? (
        <ActionDialog
          run={run.id}
          name={opened}
          onClose={() => setSearch({ action: null })}
          onChanged={() => void list.reload()}
        />
      ) : null}
    </>
  );
}
