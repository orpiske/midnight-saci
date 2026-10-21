// Where the run spent its time: totals, a timeline of each phase and unit
// on the run's time axis, and the slowest units. The data is the report's
// attempt times and timing; nothing here is estimated.
import { useId, useState } from "react";

import type { RunReport } from "../../api/types";
import { EmptyState, ErrorNotice, Loading } from "../../components/Feedback";
import { StatusIcon } from "../../components/Status";
import { formatMs, plural } from "../../lib/format";
import type { Loaded } from "../../lib/resource";
import { setSearch } from "../../lib/router";
import { OUTCOME } from "../../lib/status";
import { type AttemptSpan, type RunTime, type Totals, type UnitTime, measure, share, slowest } from "./performance";
import { depthOf, unitLook } from "./ResultsTab";

/** Units in the slowest-units table until the reader asks for all. */
export const SLOWEST_SHOWN = 10;

const count = new Intl.NumberFormat();

export function PerformanceTab({ report }: { report: Loaded<RunReport> }) {
  const data = report.resource.data;
  if (!data) {
    if (report.resource.state === "error") {
      return <ErrorNotice error={report.resource.error} title="The report could not be loaded" retry={() => void report.reload()} />;
    }
    return <Loading label="Loading the report…" />;
  }
  const run = measure(data);
  if (run.totals.attempts === 0) {
    return <EmptyState title="Nothing timed yet">No unit has started, so there is no time to show.</EmptyState>;
  }
  return (
    <div className="stack">
      <Summary run={run} active={data.lifecycle !== "finished"} />
      <Timeline run={run} />
      <Slowest run={run} />
    </div>
  );
}

function Summary({ run, active }: { run: RunTime; active: boolean }) {
  const id = useId();
  const { totals } = run;
  const tokens = totals.inputTokens + totals.outputTokens > 0;
  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <div className="panel-header">
        <h2 id={`${id}-title`}>Where the time went</h2>
        {active ? <span className="tag">So far</span> : null}
      </div>
      <div className="panel-body stack">
        <dl className="perf-tiles">
          <Tile label="Run time" value={formatMs(run.span)} />
          <Tile label="Model" value={formatMs(totals.model)} note={share(totals.model, run.span)} tone="model" />
          <Tile label="Tools" value={formatMs(totals.tools)} note={share(totals.tools, run.span)} tone="tools" />
          <Tile label="Other in units" value={formatMs(totals.other)} note={share(totals.other, run.span)} tone="other" />
          <Tile label="Between units" value={formatMs(run.outside)} note={share(run.outside, run.span)} tone="outside" />
        </dl>
        <Breakdown
          label="Run time by kind"
          parts={[
            ["model", totals.model],
            ["tools", totals.tools],
            ["other", totals.other],
            ["outside", run.outside],
          ]}
        />
        <div className="meta">
          <span className="num">{plural(totals.requests, "model request")}</span>
          {totals.requests > 0 ? <span className="num">{formatMs(totals.model / totals.requests)} per request</span> : null}
          {tokens ? (
            <span className="num">
              Tokens {count.format(totals.inputTokens)} in
              {totals.cachedTokens > 0 ? ` (${count.format(totals.cachedTokens)} cached)` : ""}, {count.format(totals.outputTokens)} out
            </span>
          ) : null}
          <span className="num">{plural(totals.attempts, "attempt")}</span>
        </div>
        {totals.untimed > 0 ? (
          <p className="muted small">
            {plural(totals.untimed, "attempt")} reported no model or tool time (still running, or a worker
            image older than the timing report). Their time counts as other time in the timeline only.
          </p>
        ) : null}
      </div>
    </section>
  );
}

function Tile({ label, value, note, tone }: { label: string; value: string; note?: string; tone?: string }) {
  return (
    <div className="perf-tile">
      <dt className="label">
        {tone ? <span className={`perf-swatch perf-${tone}`} aria-hidden="true" /> : null}
        {label}
      </dt>
      <dd className="num">
        <span className="perf-value">{value}</span>
        {note ? <span className="muted small"> {note}</span> : null}
      </dd>
    </div>
  );
}

/** A bar of parts side by side, each as wide as its share. */
function Breakdown({ label, parts }: { label: string; parts: [string, number][] }) {
  const whole = parts.reduce((sum, [, value]) => sum + value, 0);
  const text = parts.map(([kind, value]) => `${KIND[kind]} ${share(value, whole)}`).join(", ");
  return (
    <div className="perf-breakdown" role="img" aria-label={`${label}: ${text}`} title={text}>
      {parts.map(([kind, value]) =>
        value > 0 ? <span key={kind} className={`perf-${kind}`} style={{ flexGrow: value }} /> : null,
      )}
    </div>
  );
}

const KIND: Record<string, string> = {
  model: "model",
  tools: "tools",
  other: "other",
  outside: "between units",
};

function Legend() {
  return (
    <ul className="perf-legend plain-list" aria-label="Legend">
      {(["model", "tools", "other"] as const).map((kind) => (
        <li key={kind}>
          <span className={`perf-swatch perf-${kind}`} aria-hidden="true" />
          {kind === "model" ? "Model requests" : kind === "tools" ? "Tool calls" : "Other (worker, assessment)"}
        </li>
      ))}
      <li>
        <span className="perf-swatch perf-running" aria-hidden="true" />
        Running, or no timing reported
      </li>
    </ul>
  );
}

/** Ticks at quarters of the time axis. */
function Axis({ span }: { span: number }) {
  return (
    <div className="perf-row perf-axis" aria-hidden="true">
      <span />
      <div className="perf-track">
        {[0, 0.25, 0.5, 0.75, 1].map((at) => (
          <span key={at} className={`perf-tick${at === 0.25 || at === 0.75 ? " quarter" : ""}`} style={{ left: `${at * 100}%` }}>
            {formatMs(span * at)}
          </span>
        ))}
      </div>
      <span />
    </div>
  );
}

const percent = (value: number, span: number) => `${span > 0 ? (value / span) * 100 : 0}%`;

function Timeline({ run }: { run: RunTime }) {
  const id = useId();
  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <div className="panel-header">
        <h2 id={`${id}-title`}>Timeline</h2>
        <Legend />
      </div>
      <div className="panel-body perf-timeline">
        <Axis span={run.span} />
        {run.phases.map((phase, index) => {
          const ran = phase.start !== undefined && phase.end !== undefined;
          const wall = ran ? (phase.end ?? 0) - (phase.start ?? 0) : 0;
          return (
            <div key={`${index}-${phase.title}`} className="perf-phase" role="group" aria-label={`Phase ${phase.title}`}>
              <div className="perf-row perf-phase-row">
                <span className="overline perf-name">{phase.title}</span>
                <div className="perf-track">
                  {ran ? (
                    <span
                      className="perf-phase-bar"
                      style={{ left: percent(phase.start ?? 0, run.span), width: percent(wall, run.span) }}
                      role="img"
                      aria-label={`Phase ${phase.title}: ${formatMs(wall)}, ${share(wall, run.span)} of the run`}
                    />
                  ) : null}
                </div>
                <span className="num small perf-duration">{ran ? formatMs(wall) : "Did not run"}</span>
              </div>
              {phase.units.map((entry) => (
                <UnitRow key={entry.unit.key} entry={entry} span={run.span} />
              ))}
            </div>
          );
        })}
      </div>
    </section>
  );
}

function UnitRow({ entry, span }: { entry: UnitTime; span: number }) {
  const { unit } = entry;
  const look = unitLook(unit);
  return (
    <div className="perf-row">
      <button
        type="button"
        className="perf-name perf-unit"
        style={{ paddingLeft: depthOf(unit.key) * 16 }}
        onClick={() => setSearch({ tab: null, unit: unit.key, attempt: null, evidence: null }, true)}
        title={`Open ${unit.id} ${unit.title} in the results`}
      >
        <StatusIcon look={look} size={14} />
        <span className="mono small">{unit.id}</span>
        <span className="perf-title">{unit.title}</span>
      </button>
      <div className="perf-track">
        {entry.attempts.map((attempt) => (
          <AttemptBar key={attempt.number} unit={entry} attempt={attempt} span={span} />
        ))}
      </div>
      <span className="num small perf-duration">{entry.attempts.length > 0 ? formatMs(entry.totals.duration) : "—"}</span>
    </div>
  );
}

/** What an attempt bar shows, in words, for assistive technology and the tooltip. */
export function describeAttempt(attempt: AttemptSpan): string {
  const duration = attempt.end - attempt.start;
  const parts = [`Attempt ${attempt.number}`, attempt.running ? "running" : OUTCOME[attempt.outcome ?? "error"].label.toLowerCase()];
  parts.push(formatMs(duration));
  if (attempt.model !== undefined && attempt.tools !== undefined) {
    const other = Math.max(0, duration - attempt.model - attempt.tools);
    parts.push(`model ${formatMs(attempt.model)}, tools ${formatMs(attempt.tools)}, other ${formatMs(other)}`);
  }
  return parts.join(", ");
}

function AttemptBar({ unit, attempt, span }: { unit: UnitTime; attempt: AttemptSpan; span: number }) {
  const duration = attempt.end - attempt.start;
  const timed = attempt.model !== undefined && attempt.tools !== undefined;
  const model = attempt.model ?? 0;
  const tools = attempt.tools ?? 0;
  const text = `${unit.unit.id} ${describeAttempt(attempt)}`;
  return (
    <span
      className={`perf-bar${timed ? "" : " perf-running"}`}
      style={{ left: percent(attempt.start, span), width: percent(duration, span) }}
      role="img"
      aria-label={text}
      title={text}
    >
      {timed ? (
        <>
          {model > 0 ? <span className="perf-model" style={{ flexGrow: model }} /> : null}
          {tools > 0 ? <span className="perf-tools" style={{ flexGrow: tools }} /> : null}
          {duration - model - tools > 0 ? <span className="perf-other" style={{ flexGrow: duration - model - tools }} /> : null}
        </>
      ) : null}
    </span>
  );
}

function Slowest({ run }: { run: RunTime }) {
  const id = useId();
  const [all, setAll] = useState(false);
  const ran = run.units.filter((entry) => entry.attempts.length > 0).length;
  const rows = slowest(run, all ? ran : SLOWEST_SHOWN);
  return (
    <section className="stack-sm" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`}>Slowest units</h2>
      <div className="table-wrap">
        <table className="table">
          <thead>
            <tr>
              <th scope="col">Unit</th>
              <th scope="col">Phase</th>
              <th scope="col" className="num-cell">Time</th>
              <th scope="col" className="num-cell">Share</th>
              <th scope="col" className="num-cell">Model</th>
              <th scope="col" className="num-cell">Tools</th>
              <th scope="col" className="num-cell">Other</th>
              <th scope="col" className="num-cell">Requests</th>
              <th scope="col" className="num-cell">Tokens out</th>
              <th scope="col" className="num-cell">Attempts</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((entry) => (
              <SlowRow key={entry.unit.key} entry={entry} span={run.span} />
            ))}
          </tbody>
        </table>
      </div>
      {ran > SLOWEST_SHOWN ? (
        <div>
          <button type="button" className="btn" onClick={() => setAll(!all)}>
            {all ? `Show the ${SLOWEST_SHOWN} slowest` : `Show all ${ran} units`}
          </button>
        </div>
      ) : null}
    </section>
  );
}

function timed(totals: Totals, value: number): string {
  return totals.untimed === totals.attempts ? "—" : formatMs(value);
}

function SlowRow({ entry, span }: { entry: UnitTime; span: number }) {
  const { unit, totals } = entry;
  return (
    <tr>
      <td className="title-cell">
        <button
          type="button"
          className="link-btn"
          onClick={() => setSearch({ tab: null, unit: unit.key, attempt: null, evidence: null }, true)}
        >
          <span className="mono small">{unit.id}</span> {unit.title}
        </button>
      </td>
      <td>{entry.phase}</td>
      <td className="num num-cell">{formatMs(totals.duration)}</td>
      <td className="num num-cell">{share(totals.duration, span)}</td>
      <td className="num num-cell">{timed(totals, totals.model)}</td>
      <td className="num num-cell">{timed(totals, totals.tools)}</td>
      <td className="num num-cell">{timed(totals, totals.other)}</td>
      <td className="num num-cell">{totals.untimed === totals.attempts ? "—" : totals.requests}</td>
      <td className="num num-cell">{totals.outputTokens > 0 ? count.format(totals.outputTokens) : "—"}</td>
      <td className="num num-cell">{totals.attempts}</td>
    </tr>
  );
}
