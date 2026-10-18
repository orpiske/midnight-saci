import { seg } from "../../api/client";
import type { Action } from "../../api/types";
import { Badge } from "../../components/Status";
import { formatRelative, formatTime, shortId } from "../../lib/format";
import { Link } from "../../lib/router";
import { ACTION } from "../../lib/status";

/** Actions as rows; `open` shows one in the action dialog. */
export function ActionsTable({
  actions,
  caption,
  open,
  showRun,
}: {
  actions: Action[];
  caption: string;
  open: (action: Action) => void;
  showRun: boolean;
}) {
  return (
    <div className="table-wrap">
      <table className="table">
        <caption className="visually-hidden">{caption}</caption>
        <thead>
          <tr>
            <th scope="col">Action</th>
            <th scope="col">State</th>
            {showRun ? <th scope="col">Run</th> : null}
            <th scope="col">Sends</th>
            <th scope="col">Attempts</th>
            <th scope="col">Updated</th>
          </tr>
        </thead>
        <tbody>
          {actions.map((action) => (
            <tr key={`${action.run_id}/${action.name}`}>
              <td className="title-cell">
                <button type="button" className="link-btn mono" onClick={() => open(action)}>
                  {action.name}
                </button>
              </td>
              <td>
                <Badge look={ACTION[action.state]} />
              </td>
              {showRun ? (
                <td>
                  <Link to={`/runs/${seg(action.run_id)}?tab=actions`} className="mono small" title={action.run_id}>
                    {shortId(action.run_id)}
                  </Link>
                </td>
              ) : null}
              <td className="mono small">
                {action.server} / {action.tool}
              </td>
              <td className="num">{action.attempts}</td>
              <td className="num" title={formatTime(action.updated_at)}>
                {formatRelative(action.updated_at)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
