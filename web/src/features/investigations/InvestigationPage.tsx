// One investigation: its state, and once the agent concluded, the summary,
// the hypotheses with the code they cite and their evidence, the cloned
// commits and the bug. The agent's evidence comes from the report of the
// run that executes the investigation, which has the same ID.
import { ExternalLink, RefreshCw } from "lucide-react";
import { useCallback, useId } from "react";

import { ApiError, seg } from "../../api/client";
import type { Evidence, Hypothesis, Investigation, InvestigationReport, RunReport } from "../../api/types";
import { CopyText } from "../../components/Copy";
import { Callout, ErrorNotice, Loading } from "../../components/Feedback";
import { Badge } from "../../components/Status";
import { between, formatTime, plural } from "../../lib/format";
import { type Loaded, useInterval, useResource } from "../../lib/resource";
import { Link, useLocation } from "../../lib/router";
import { CONFIDENCE, investigationLook } from "../../lib/status";
import { SafeText } from "../../lib/text";
import { useClient } from "../connection/connection";
import { EvidenceView } from "../evidence/EvidenceView";
import { Cite } from "../runs/ResultsTab";
import { codeLink, httpsLink, lineRange } from "./links";

/** How often an active investigation refreshes. */
const REFRESH_MS = 3000;

export function InvestigationPage({ id }: { id: string }) {
  const client = useClient();
  const { search } = useLocation();
  const investigation = useResource(
    (signal) => client.get<Investigation>(`/investigations/${seg(id)}`, { signal }),
    [client, id],
  );
  const report = useResource(
    (signal) => client.get<RunReport>(`/runs/${seg(id)}/report`, { signal }),
    [client, id],
  );
  const refresh = useCallback(() => {
    void investigation.reload();
    void report.reload();
  }, [investigation.reload, report.reload]);
  const data = investigation.resource.data;
  useInterval(refresh, REFRESH_MS, data !== undefined && data.lifecycle !== "finished");

  if (investigation.resource.state === "loading") {
    return (
      <div className="page">
        <Loading label="Loading the investigation…" />
      </div>
    );
  }
  if (!data) {
    const error = investigation.resource.error;
    const missing = error instanceof ApiError && error.status === 404;
    return (
      <div className="page stack">
        <Breadcrumb />
        <ErrorNotice
          error={error}
          title={missing ? "No such investigation" : "The investigation could not be loaded"}
          retry={missing ? undefined : () => void investigation.reload()}
        />
      </div>
    );
  }
  return (
    <div className="page stack">
      <Breadcrumb parent={data.parent_run} unit={data.unit} />
      <Header investigation={data} reload={refresh} />
      <Result investigation={data} report={report} selected={search.get("evidence")} />
      <Sources investigation={data} />
      <AgentEvidence report={report} selected={search.get("evidence")} cited={citedEvidence(data)} />
    </div>
  );
}

function Breadcrumb({ parent, unit }: { parent?: string; unit?: string }) {
  return (
    <nav className="breadcrumb" aria-label="Breadcrumb">
      <Link to="/runs">Runs</Link>
      <span aria-hidden="true">/</span>
      {parent ? (
        <>
          <Link to={`/runs/${seg(parent)}?unit=${encodeURIComponent(unit ?? "")}`}>Run</Link>
          <span aria-hidden="true">/</span>
        </>
      ) : null}
      <span aria-current="page">Investigation</span>
    </nav>
  );
}

function Header({ investigation, reload }: { investigation: Investigation; reload: () => void }) {
  const { id, parent_run, unit, lifecycle, outcome, reason, created_at, finished_at } = investigation;
  return (
    <header className="run-header">
      <div className="page-header">
        <div className="stack-sm">
          <h1>Investigation of unit {unit}</h1>
          <div className="row">
            <Badge look={investigationLook(lifecycle, outcome)} />
          </div>
        </div>
        <div className="actions">
          <button type="button" className="btn" onClick={reload}>
            <RefreshCw size={16} aria-hidden="true" />
            Refresh
          </button>
          <Link to={`/runs/${seg(id)}?tab=activity`} className="btn">
            Agent activity
          </Link>
        </div>
      </div>
      {reason && outcome !== "concluded" ? (
        <p>
          <span className="muted">Why: </span>
          <SafeText text={reason} />
        </p>
      ) : null}
      <div className="meta">
        <span>
          Investigation <CopyText value={id} title="investigation ID" />
        </span>
        <span>
          Run <Link to={`/runs/${seg(parent_run)}?unit=${encodeURIComponent(unit)}`} className="mono">{parent_run}</Link>
        </span>
        <span className="num">Created {formatTime(created_at)}</span>
        {finished_at ? (
          <span className="num">
            Finished {formatTime(finished_at)} ({between(created_at, finished_at)})
          </span>
        ) : null}
      </div>
    </header>
  );
}

/** What to show before, or instead of, the agent's result. */
function pending(investigation: Investigation): string {
  if (investigation.lifecycle !== "finished") {
    return "The agent is reading the code. The result appears here when it concludes.";
  }
  if (investigation.outcome === "cancelled") return "The investigation was cancelled before the agent concluded.";
  return "The investigation ended without a result. The reason above tells why.";
}

function Result({
  investigation,
  report,
  selected,
}: {
  investigation: Investigation;
  report: Loaded<RunReport>;
  selected: string | null;
}) {
  const id = useId();
  const result = investigation.result;
  const evidence = report.resource.data?.evidence ?? [];
  return (
    <section className="panel" aria-labelledby={`${id}-result`}>
      <div className="panel-header">
        <h2 id={`${id}-result`}>Result</h2>
      </div>
      <div className="panel-body stack">
        {result ? (
          <>
            <p className="prewrap">
              <SafeText text={result.summary} />
            </p>
            <Bug result={result} />
            <Hypotheses investigation={investigation} result={result} evidence={evidence} selected={selected} />
          </>
        ) : (
          <p className="muted">{pending(investigation)}</p>
        )}
      </div>
    </section>
  );
}

function Bug({ result }: { result: InvestigationReport }) {
  if (result.bug) {
    const url = httpsLink(result.bug.url);
    return (
      <Callout tone="neutral" title="Bug">
        {url ? (
          <a href={url} target="_blank" rel="noopener noreferrer">
            <SafeText text={result.bug.title} /> <ExternalLink size={14} aria-hidden="true" />
          </a>
        ) : (
          <>
            <SafeText text={result.bug.title} /> (<span className="mono">{result.bug.url}</span>)
          </>
        )}
      </Callout>
    );
  }
  if (result.no_bug_reason) {
    return (
      <Callout tone="neutral" title="No bug filed">
        <SafeText text={result.no_bug_reason} />
      </Callout>
    );
  }
  return null;
}

function Hypotheses({
  investigation,
  result,
  evidence,
  selected,
}: {
  investigation: Investigation;
  result: InvestigationReport;
  evidence: Evidence[];
  selected: string | null;
}) {
  const hypotheses = result.hypotheses ?? [];
  if (hypotheses.length === 0) return <p className="muted small">The agent gave no hypothesis.</p>;
  const byId = new Map(evidence.map((e) => [e.id, e]));
  const shown = selected ? byId.get(selected) : undefined;
  const cited = selected !== null && citedEvidence(investigation).has(selected);
  return (
    <div className="stack-sm">
      <span className="label">
        {plural(hypotheses.length, "hypothesis", "hypotheses")}: probable causes, not proven ones
      </span>
      <ol className="criteria">
        {hypotheses.map((hypothesis, index) => (
          <HypothesisItem
            key={index}
            hypothesis={hypothesis}
            investigation={investigation}
            byId={byId}
            selected={selected}
          />
        ))}
      </ol>
      {shown && cited ? <EvidenceView evidence={shown} /> : null}
    </div>
  );
}

function HypothesisItem({
  hypothesis,
  investigation,
  byId,
  selected,
}: {
  hypothesis: Hypothesis;
  investigation: Investigation;
  byId: Map<string, Evidence>;
  selected: string | null;
}) {
  const code = hypothesis.code ?? [];
  const evidence = hypothesis.evidence ?? [];
  return (
    <li className="criterion stack-sm">
      <div className="row-between">
        <span className="prewrap">
          <SafeText text={hypothesis.statement} />
        </span>
        <span className="tag">{CONFIDENCE[hypothesis.confidence]}</span>
      </div>
      {code.length > 0 ? (
        <ul className="plain-list stack-sm">
          {code.map((location, index) => {
            const text = `${location.repository} ${location.path}:${lineRange(location)}`;
            const url = codeLink(location, investigation.repositories);
            return (
              <li key={index} className="row small">
                {url ? (
                  <a href={url} target="_blank" rel="noopener noreferrer" className="mono">
                    {text} <ExternalLink size={14} aria-hidden="true" />
                  </a>
                ) : (
                  <span className="mono">{text}</span>
                )}
                <span className="row nowrap muted">
                  at <CopyText value={location.commit} label={location.commit.slice(0, 12)} title="commit" />
                </span>
              </li>
            );
          })}
        </ul>
      ) : null}
      {evidence.length > 0 ? (
        <div className="row">
          {evidence.map((id) => (
            <Cite key={id} id={id} evidence={byId.get(id)} selected={selected === id} />
          ))}
        </div>
      ) : null}
    </li>
  );
}

function Sources({ investigation }: { investigation: Investigation }) {
  const id = useId();
  const cloned = new Map((investigation.result?.sources ?? []).map((s) => [s.name, s.commit]));
  return (
    <section className="panel" aria-labelledby={`${id}-sources`}>
      <div className="panel-header">
        <h2 id={`${id}-sources`}>Source code</h2>
      </div>
      <div className="table-wrap">
        <table className="table">
          <thead>
            <tr>
              <th scope="col">Repository</th>
              <th scope="col">Ref</th>
              <th scope="col">Cloned commit</th>
              <th scope="col">URL</th>
            </tr>
          </thead>
          <tbody>
            {investigation.repositories.map((repository) => {
              const commit = cloned.get(repository.name);
              return (
                <tr key={repository.name}>
                  <th scope="row">{repository.name}</th>
                  <td className="mono">{repository.ref}</td>
                  <td>
                    {commit ? (
                      <CopyText value={commit} label={commit.slice(0, 12)} title={`${repository.name} commit`} />
                    ) : (
                      <span className="muted">Not reported</span>
                    )}
                  </td>
                  <td className="mono small">{repository.url}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}

/** Every observation of the agent: the clones, the prepare commands and
 * the tool calls. */
function AgentEvidence({
  report,
  selected,
  cited,
}: {
  report: Loaded<RunReport>;
  selected: string | null;
  cited: Set<string>;
}) {
  const id = useId();
  const data = report.resource.data;
  const evidence = data?.evidence ?? [];
  // Evidence that a hypothesis cites opens under the hypotheses.
  const shown = evidence.find((e) => e.id === selected && !cited.has(e.id));
  return (
    <section className="panel" aria-labelledby={`${id}-evidence`}>
      <div className="panel-header">
        <h2 id={`${id}-evidence`}>Agent evidence ({evidence.length})</h2>
      </div>
      <div className="panel-body stack">
        {!data ? (
          report.resource.state === "error" ? (
            <ErrorNotice error={report.resource.error} title="The evidence could not be loaded" retry={() => void report.reload()} />
          ) : (
            <Loading label="Loading the evidence…" />
          )
        ) : evidence.length === 0 ? (
          <p className="muted small">The agent has recorded no tool calls yet.</p>
        ) : (
          <div className="row">
            {evidence.map((e) => (
              <Cite key={e.id} id={e.id} evidence={e} selected={selected === e.id} />
            ))}
          </div>
        )}
        {shown ? <EvidenceView evidence={shown} /> : null}
      </div>
    </section>
  );
}

/** The evidence IDs that the hypotheses cite. */
function citedEvidence(investigation: Investigation): Set<string> {
  return new Set((investigation.result?.hypotheses ?? []).flatMap((h) => h.evidence ?? []));
}
