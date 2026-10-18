// The plan a run executes, from the documents retained with it: each
// source document and the units the backend expanded from them.
import { useId } from "react";

import type { PlannedUnit, RunPlan } from "../../api/types";
import { CodeView } from "../../components/Code";
import { Callout, ErrorNotice, Loading } from "../../components/Feedback";
import type { Loaded } from "../../lib/resource";
import { setSearch } from "../../lib/router";
import { SafeText } from "../../lib/text";

function documentOrder(plan: RunPlan): string[] {
  const rest = Object.keys(plan.documents)
    .filter((path) => path !== plan.entry)
    .sort();
  return [plan.entry, ...rest];
}

export function PlanTab({ plan, search }: { plan: Loaded<RunPlan>; search: URLSearchParams }) {
  const id = useId();
  const data = plan.resource.data;
  if (!data) {
    if (plan.resource.state === "error") {
      return <ErrorNotice error={plan.resource.error} title="The plan could not be loaded" retry={() => void plan.reload()} />;
    }
    return <Loading label="Loading the plan…" />;
  }
  const documents = documentOrder(data);
  const wanted = search.get("document");
  const document = wanted && wanted in data.documents ? wanted : data.entry;
  const line = Number(search.get("line")) || null;
  const marked = new Set(
    (data.diagnostics ?? []).filter((d) => d.document === document).map((d) => d.line),
  );
  const show = (path: string, at: number | null) =>
    setSearch({ document: path === data.entry ? null : path, line: at ? String(at) : null });

  return (
    <div className="stack">
      {data.diagnostics?.length ? (
        <Callout tone="warn" title="The retained plan no longer expands">
          <ul>
            {data.diagnostics.map((d, index) => (
              <li key={index}>
                <span className="mono">
                  {d.document}:{d.line}
                </span>{" "}
                <SafeText text={d.message} />
              </li>
            ))}
          </ul>
        </Callout>
      ) : null}
      <section className="panel" aria-labelledby={`${id}-units`}>
        <div className="panel-header">
          <h2 id={`${id}-units`}>Units in execution order</h2>
        </div>
        <UnitsTable units={data.units} show={show} />
      </section>
      <section className="panel" aria-labelledby={`${id}-source`}>
        <div className="panel-header">
          <h2 id={`${id}-source`}>Source</h2>
          <label className="field row">
            <span className="label">Document</span>
            <select value={document} onChange={(event) => show(event.target.value, null)}>
              {documents.map((path) => (
                <option key={path} value={path}>
                  {path === data.entry ? `${path} (entry)` : path}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="panel-body">
          <CodeView
            text={data.documents[document] ?? ""}
            label={`Source of ${document}`}
            highlight={line}
            diagnostics={marked}
            maxHeight={560}
          />
        </div>
      </section>
    </div>
  );
}

function UnitsTable({
  units,
  show,
}: {
  units: PlannedUnit[];
  show: (document: string, line: number | null) => void;
}) {
  if (units.length === 0) return <p className="panel-body muted">No units.</p>;
  return (
    <div className="table-wrap">
      <table className="table">
        <thead>
          <tr>
            <th scope="col">Key</th>
            <th scope="col">Unit</th>
            <th scope="col">Phase</th>
            <th scope="col">Expected results</th>
            <th scope="col">Source</th>
          </tr>
        </thead>
        <tbody>
          {units.map((unit) => (
            <tr key={unit.key}>
              <td className="mono small">{unit.key}</td>
              <td>
                {unit.kind === "test" ? "Test" : "Step"} {unit.id}: {unit.title}
                {unit.cleanup ? <span className="tag" style={{ marginLeft: 8 }}>Cleanup</span> : null}
              </td>
              <td>{unit.phase}</td>
              <td className="num">{unit.criteria.length}</td>
              <td>
                <button
                  type="button"
                  className="link-btn mono small"
                  onClick={() => show(unit.location.document, unit.location.line)}
                >
                  {unit.location.document}:{unit.location.line}
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
