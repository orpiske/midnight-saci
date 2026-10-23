// A run's units as the report records them: outcome, attempts, criteria
// with their basis and cited evidence, and the evidence itself. The
// selected unit, attempt and evidence live in the URL so a link reopens
// the same view.
import { Search } from "lucide-react";
import { useId } from "react";

import type {
  Attempt,
  CriterionResult,
  Evidence,
  ExecutionError,
  Investigation,
  PlannedUnit,
  RunPlan,
  RunReport,
  UnitReport,
} from "../../api/types";
import { CodeView } from "../../components/Code";
import { Callout, EmptyState, ErrorNotice, Loading } from "../../components/Feedback";
import { Badge, StatusIcon } from "../../components/Status";
import { Tabs, tabPanelProps } from "../../components/Tabs";
import { between, formatTime, plural } from "../../lib/format";
import type { Loaded } from "../../lib/resource";
import { Link, setSearch } from "../../lib/router";
import {
  BASIS,
  CRITERION,
  IN_PROGRESS,
  OUTCOME,
  PENDING,
  type StatusLook,
  investigationLook,
} from "../../lib/status";
import { SafeText } from "../../lib/text";
import { EvidenceView, evidenceSummary } from "../evidence/EvidenceView";

const ERROR_KIND: Record<ExecutionError["kind"], string> = {
  provider: "Model provider failed",
  invalid_assessment: "The model's assessment was invalid",
  tool: "Tool failure",
  limit_exceeded: "A limit was exceeded",
  launch_failed: "The worker could not start",
  worker_lost: "The worker was lost",
  invalid_specification: "The run specification was invalid",
};

export function unitLook(unit: UnitReport): StatusLook {
  if (unit.outcome) return OUTCOME[unit.outcome];
  const last = unit.attempts[unit.attempts.length - 1];
  return last && !last.finished_at ? IN_PROGRESS : PENDING;
}

/** How deep a unit is nested in procedure invocations. */
export const depthOf = (key: string): number => key.split(">").length - 1;

function unitSub(unit: UnitReport): string {
  const parts: string[] = [];
  if (unit.outcome && unit.attempts.length > 1) {
    parts.push(`${OUTCOME[unit.outcome].label} · ${plural(unit.attempts.length, "attempt")}`);
  }
  if (unit.cleanup) parts.push("Cleanup");
  return parts.join(" · ");
}

function UnitList({
  report,
  selected,
}: {
  report: RunReport;
  selected: string;
}) {
  const byKey = new Map(report.units.map((unit) => [unit.key, unit]));
  let phase: string | null = null;
  const rows = report.units.map((unit) => {
    const root = byKey.get(unit.key.split(">")[0] ?? unit.key) ?? unit;
    const heading = root.phase !== phase ? root.phase : null;
    phase = root.phase;
    const look = unitLook(unit);
    const sub = unitSub(unit);
    return (
      <li key={unit.key}>
        {heading !== null ? <div className="unit-phase overline">{heading}</div> : null}
        <button
          type="button"
          className="unit-item"
          aria-current={unit.key === selected ? "true" : undefined}
          style={{ paddingLeft: 8 + depthOf(unit.key) * 16 }}
          onClick={() => setSearch({ unit: unit.key, attempt: null, evidence: null })}
        >
          <StatusIcon look={look} />
          <span className="unit-name">
            <span className="mono small">{unit.id}</span> {unit.title}
            {sub ? <span className="unit-sub">{sub}</span> : null}
          </span>
        </button>
      </li>
    );
  });
  return (
    <nav className="panel unit-list" aria-label="Units">
      <ul className="plain-list">{rows}</ul>
    </nav>
  );
}

export function ResultsTab({
  report,
  plan,
  investigations,
  search,
}: {
  report: Loaded<RunReport>;
  plan: RunPlan | undefined;
  /** The run's investigations, oldest first. */
  investigations: Investigation[];
  search: URLSearchParams;
}) {
  const data = report.resource.data;
  if (!data) {
    if (report.resource.state === "error") {
      return <ErrorNotice error={report.resource.error} title="The report could not be loaded" retry={() => void report.reload()} />;
    }
    return <Loading label="Loading the report…" />;
  }
  if (data.units.length === 0) {
    return <EmptyState title="No units yet">The run has not reported its units.</EmptyState>;
  }
  const wanted = search.get("unit");
  const unit =
    data.units.find((u) => u.key === wanted) ??
    data.units.find((u) => u.outcome && u.outcome !== "passed" && u.outcome !== "skipped") ??
    data.units.find((u) => !u.outcome) ??
    data.units[0];
  if (!unit) return null;
  const notRun = data.cleanup_not_run
    .map((key) => data.units.find((u) => u.key === key))
    .filter((u): u is UnitReport => u !== undefined);
  return (
    <div className="stack">
      {notRun.length > 0 ? (
        <Callout tone="warn" title="Cleanup did not run">
          These cleanup units never ran, so resources they would remove may still exist on the
          target: {notRun.map((u) => `${u.id} ${u.title}`).join(", ")}.
        </Callout>
      ) : null}
      <div className="split" style={{ ["--units-width" as string]: "300px" }}>
        <UnitList report={data} selected={unit.key} />
        <UnitDetail
          unit={unit}
          planned={plan?.units.find((p) => p.key === unit.key)}
          evidence={data.evidence}
          investigation={investigations.findLast((i) => i.unit === unit.key)}
          search={search}
        />
      </div>
    </div>
  );
}

function UnitDetail({
  unit,
  planned,
  evidence,
  investigation,
  search,
}: {
  unit: UnitReport;
  planned: PlannedUnit | undefined;
  evidence: Evidence[];
  /** The unit's latest investigation. */
  investigation: Investigation | undefined;
  search: URLSearchParams;
}) {
  const id = useId();
  const numbers = unit.attempts.map((a) => a.number);
  const asked = Number(search.get("attempt"));
  const number = numbers.includes(asked) ? asked : numbers[numbers.length - 1];
  const attempt = unit.attempts.find((a) => a.number === number);
  const kind = unit.kind === "test" ? "Test" : "Step";
  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <div className="panel-header">
        <div className="stack-sm">
          <h2 id={`${id}-title`}>
            {kind} {unit.id}: {unit.title}
          </h2>
          <div className="row">
            <Badge look={unitLook(unit)} />
            {unit.cleanup ? <span className="tag">Cleanup</span> : null}
            <span className="muted small mono">
              {unit.location.document}:{unit.location.line}
            </span>
          </div>
        </div>
        {investigation ? (
          <Link to={`/investigations/${encodeURIComponent(investigation.id)}`} className="btn">
            <Search size={16} aria-hidden="true" />
            Investigation: {investigationLook(investigation.lifecycle, investigation.outcome).label}
          </Link>
        ) : null}
      </div>
      <div className="panel-body stack">
        {unit.reason ? (
          <Callout tone="neutral" title={unit.outcome === "skipped" ? "Skipped" : "Interrupted"}>
            <SafeText text={unit.reason} />
          </Callout>
        ) : null}
        {planned?.invoked_by?.length ? (
          <p className="muted small">
            Reached through {planned.invoked_by.join(" → ")}.
          </p>
        ) : null}
        {planned ? (
          <details className="authored">
            <summary>Action as authored</summary>
            <div className="stack-sm">
              <CodeView text={planned.action} label="Action as authored" wrap />
              {planned.verification ? (
                <>
                  <span className="label">Verification</span>
                  <CodeView text={planned.verification} label="Verification as authored" wrap />
                </>
              ) : null}
            </div>
          </details>
        ) : null}
        {unit.attempts.length === 0 || !attempt ? (
          <p className="muted">This unit has not started.</p>
        ) : (
          <>
            {unit.attempts.length > 1 ? (
              <Tabs
                id={id}
                label="Attempts"
                small
                selected={String(attempt.number)}
                onSelect={(key) => setSearch({ attempt: key, evidence: null })}
                items={unit.attempts.map((a) => ({
                  key: String(a.number),
                  label: `Attempt ${a.number}${a.outcome ? ` · ${OUTCOME[a.outcome].label}` : ""}`,
                }))}
              />
            ) : null}
            <div {...(unit.attempts.length > 1 ? tabPanelProps(id, String(attempt.number)) : {})}>
              <AttemptView
                unit={unit}
                planned={planned}
                attempt={attempt}
                evidence={evidence}
                selected={search.get("evidence")}
              />
            </div>
          </>
        )}
      </div>
    </section>
  );
}

export function Cite({ id, evidence, selected }: { id: string; evidence: Evidence | undefined; selected: boolean }) {
  return (
    <button
      type="button"
      className="cite"
      aria-pressed={selected}
      onClick={() => setSearch({ evidence: selected ? null : id })}
    >
      <span className="mono">{id}</span>
      <span>{evidence ? evidenceSummary(evidence) : "not in the report"}</span>
    </button>
  );
}

function AttemptView({
  unit,
  planned,
  attempt,
  evidence,
  selected,
}: {
  unit: UnitReport;
  planned: PlannedUnit | undefined;
  attempt: Attempt;
  evidence: Evidence[];
  selected: string | null;
}) {
  const byId = new Map(evidence.map((e) => [e.id, e]));
  const criteria = unit.criteria ?? planned?.criteria ?? [];
  const results = new Map<number, CriterionResult>(
    (attempt.assessment?.criteria ?? []).map((r) => [r.criterion, r]),
  );
  const shown = selected ? byId.get(selected) : undefined;
  return (
    <div className="stack">
      <div className="meta">
        <span className="num">Started {formatTime(attempt.started_at)}</span>
        {attempt.finished_at ? <span className="num">Took {between(attempt.started_at, attempt.finished_at)}</span> : <span>Still running</span>}
        {attempt.outcome ? <span>Outcome {OUTCOME[attempt.outcome].label}</span> : null}
      </div>
      {attempt.error ? (
        <Callout tone="fail" title={ERROR_KIND[attempt.error.kind]}>
          <SafeText text={attempt.error.message} />
        </Callout>
      ) : null}
      {attempt.assessment ? (
        <div className="stack-sm">
          <span className="label">Assessment</span>
          <p className="prewrap">
            <SafeText text={attempt.assessment.explanation} />
          </p>
        </div>
      ) : null}
      <div className="stack-sm">
        <span className="label">Expected results</span>
        {criteria.length === 0 ? (
          <p className="muted small">The unit lists no expected results.</p>
        ) : (
          <ol className="criteria">
            {criteria.map((criterion) => {
              const result = results.get(criterion.index);
              return (
                <li key={criterion.index} className="criterion stack-sm">
                  <div className="row-between">
                    <span>
                      <SafeText text={criterion.text} />
                    </span>
                    {result ? <Badge look={CRITERION[result.status]} /> : <span className="tag">Not assessed</span>}
                  </div>
                  {result ? (
                    <>
                      <span className="muted small">{BASIS[result.basis]}</span>
                      <p className="prewrap small">
                        <SafeText text={result.explanation} />
                      </p>
                      {result.evidence.length > 0 ? (
                        <div className="row">
                          {result.evidence.map((id) => (
                            <Cite key={id} id={id} evidence={byId.get(id)} selected={selected === id} />
                          ))}
                        </div>
                      ) : (
                        <span className="muted small">No evidence cited.</span>
                      )}
                    </>
                  ) : null}
                </li>
              );
            })}
          </ol>
        )}
      </div>
      <div className="stack-sm">
        <span className="label">Evidence ({attempt.evidence.length})</span>
        {attempt.evidence.length === 0 ? (
          <p className="muted small">No tool calls were recorded in this attempt.</p>
        ) : (
          <div className="row">
            {attempt.evidence.map((id) => (
              <Cite key={id} id={id} evidence={byId.get(id)} selected={selected === id} />
            ))}
          </div>
        )}
      </div>
      {shown ? <EvidenceView evidence={shown} /> : null}
    </div>
  );
}
