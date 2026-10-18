// One recorded observation, rendered faithfully: commands, requests and
// results exactly as retained, with truncation and byte counts, redaction
// markers and control characters visible. Nothing is fetched from or
// linked to the recorded URLs.
import { useId, useState } from "react";

import type {
  CapturedText,
  Evidence,
  HttpEvidence,
  McpEvidence,
  RejectedEvidence,
  ShellEvidence,
} from "../../api/types";
import { CodeView } from "../../components/Code";
import { Callout } from "../../components/Feedback";
import { Tabs, tabPanelProps } from "../../components/Tabs";
import { formatBytes, formatMs, formatTime } from "../../lib/format";
import { SafeText } from "../../lib/text";

/** A one-line description for citations and lists. */
export function evidenceSummary(evidence: Evidence): string {
  switch (evidence.tool) {
    case "shell": {
      const command = evidence.command.split("\n")[0] ?? "";
      const end =
        evidence.exit_code !== undefined && evidence.exit_code !== null
          ? `exit ${evidence.exit_code}`
          : TERMINATION[evidence.termination];
      return `$ ${command.length > 60 ? `${command.slice(0, 60)}…` : command} · ${end}`;
    }
    case "http":
      return `${evidence.method} ${evidence.url} · ${
        evidence.response ? evidence.response.status : "no response"
      }`;
    case "mcp":
      return `${evidence.server} / ${evidence.tool_name}${
        evidence.protocol_error ? " · no result" : evidence.is_error ? " · tool error" : ""
      }`;
    case "rejected":
      return `Rejected ${evidence.name}`;
  }
}

export const TOOL_LABEL: Record<Evidence["tool"], string> = {
  shell: "Shell command",
  http: "HTTP request",
  mcp: "MCP tool call",
  rejected: "Rejected tool call",
};

const TERMINATION: Record<ShellEvidence["termination"], string> = {
  exited: "exited",
  timed_out: "timed out; the process tree was stopped",
  cancelled: "cancelled; the process tree was stopped",
  session_ended: "ended the shell session",
  session_lost: "shell session lost",
};

export function Captured({ text, label }: { text: CapturedText; label: string }) {
  return (
    <div className="stack-sm">
      <div className="row-between">
        <span className="label">{label}</span>
        <span className="muted small num">
          {text.truncated
            ? `Truncated: showing ${formatBytes(new TextEncoder().encode(text.text).length)} of ${formatBytes(text.total_bytes)}`
            : formatBytes(text.total_bytes)}
        </span>
      </div>
      <CodeView text={text.text} label={label} emptyLabel="No output" wrap maxHeight={360} />
      {text.truncated ? (
        <span className="muted small">
          Saci kept the first part of this output, up to the configured limit, and discarded the
          rest.
        </span>
      ) : null}
    </div>
  );
}

function Headers({ headers, label }: { headers: Record<string, string> | undefined; label: string }) {
  const entries = Object.entries(headers ?? {});
  if (entries.length === 0) return <p className="muted small">No {label.toLowerCase()}.</p>;
  return (
    <div className="stack-sm">
      <span className="label">{label}</span>
      <dl className="kv mono small">
        {entries.map(([name, value]) => (
          <div key={name} style={{ display: "contents" }}>
            <dt>{name}</dt>
            <dd>
              <SafeText text={value} />
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

function Shell({ evidence }: { evidence: ShellEvidence }) {
  const exited = evidence.exit_code !== undefined && evidence.exit_code !== null;
  return (
    <div className="stack">
      <CodeView text={evidence.command} label="Command" wrap />
      <dl className="kv">
        <dt>Ended</dt>
        <dd>
          {exited ? (
            <span className="mono">exit {evidence.exit_code}</span>
          ) : (
            TERMINATION[evidence.termination]
          )}
        </dd>
      </dl>
      {evidence.restarted ? (
        <Callout tone="warn" title="New shell session">
          The previous session was lost or ended, so this command ran in a fresh shell. Variables,
          functions and the working directory from earlier commands were gone.
        </Callout>
      ) : null}
      {evidence.session_lost ? (
        <Callout tone="warn" title="Shell session lost">
          The shell session did not survive this command; the next command starts a new one.
        </Callout>
      ) : null}
      <Captured text={evidence.stdout} label="Standard output" />
      <Captured text={evidence.stderr} label="Standard error" />
    </div>
  );
}

type HttpTab = "headers" | "body";

function Http({ evidence }: { evidence: HttpEvidence }) {
  const id = useId();
  const [tab, setTab] = useState<HttpTab>("body");
  const response = evidence.response;
  return (
    <div className="stack">
      <CodeView text={`${evidence.method} ${evidence.url}`} label="Request line" wrap />
      {response ? (
        <dl className="kv">
          <dt>Status</dt>
          <dd className="mono">{response.status}</dd>
        </dl>
      ) : (
        <Callout tone="fail" title="No response" role="status">
          The request failed before a response arrived: <SafeText text={evidence.transport_error ?? "unknown transport failure"} />.
          This is a transport failure, not an answer from the target.
        </Callout>
      )}
      {response?.body_error ? (
        <Callout tone="warn" title="Incomplete body">
          The status and headers arrived, but reading the body failed: <SafeText text={response.body_error} />
        </Callout>
      ) : null}
      <Tabs
        id={id}
        label="HTTP exchange"
        small
        selected={tab}
        onSelect={setTab}
        items={[
          { key: "body", label: "Body" },
          { key: "headers", label: "Headers" },
        ]}
      />
      <div {...tabPanelProps(id, tab)} className="stack">
        {tab === "headers" ? (
          <>
            <Headers headers={evidence.request_headers} label="Request headers" />
            {response ? <Headers headers={response.headers} label="Response headers" /> : null}
          </>
        ) : (
          <>
            {evidence.request_body ? <Captured text={evidence.request_body} label="Request body" /> : null}
            {response ? <Captured text={response.body} label="Response body" /> : null}
            {!evidence.request_body && !response ? <p className="muted small">No bodies.</p> : null}
          </>
        )}
      </div>
    </div>
  );
}

function Mcp({ evidence }: { evidence: McpEvidence }) {
  return (
    <div className="stack">
      <dl className="kv">
        <dt>Server</dt>
        <dd className="mono">{evidence.server}</dd>
        <dt>Tool</dt>
        <dd className="mono">{evidence.tool_name}</dd>
      </dl>
      <div className="stack-sm">
        <span className="label">Arguments</span>
        <CodeView text={JSON.stringify(evidence.arguments, null, 2)} label="Arguments" wrap />
      </div>
      {evidence.protocol_error ? (
        <Callout tone="fail" title="No result">
          The call failed, timed out or was cancelled before the server answered:{" "}
          <SafeText text={evidence.protocol_error} />
        </Callout>
      ) : null}
      {evidence.is_error ? (
        <Callout tone="warn" title="The tool reported an error">
          The server answered, and marked its result as an error.
        </Callout>
      ) : null}
      <Captured text={evidence.result} label="Result" />
    </div>
  );
}

function Rejected({ evidence }: { evidence: RejectedEvidence }) {
  return (
    <div className="stack">
      <Callout tone="neutral" title="Never ran">
        Saci rejected this tool request before it ran: <SafeText text={evidence.reason} />
      </Callout>
      <dl className="kv">
        <dt>Requested tool</dt>
        <dd className="mono">
          <SafeText text={evidence.name} />
        </dd>
      </dl>
      <div className="stack-sm">
        <span className="label">Arguments as received</span>
        <CodeView text={evidence.arguments} label="Arguments as received" wrap />
      </div>
    </div>
  );
}

type View = "details" | "raw";

export function EvidenceView({ evidence }: { evidence: Evidence }) {
  const id = useId();
  const [view, setView] = useState<View>("details");
  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <div className="panel-header">
        <div className="stack-sm">
          <h3 id={`${id}-title`}>
            <span className="mono">{evidence.id}</span> · {TOOL_LABEL[evidence.tool]}
          </h3>
          <div className="meta">
            <span>Attempt {evidence.attempt}</span>
            <span className="num">{formatMs(evidence.duration_ms)}</span>
            <span className="num">Recorded {formatTime(evidence.recorded_at)}</span>
          </div>
        </div>
      </div>
      <div className="panel-body">
        <Tabs
          id={id}
          label="Evidence view"
          small
          selected={view}
          onSelect={setView}
          items={[
            { key: "details", label: "Details" },
            { key: "raw", label: "Raw JSON" },
          ]}
        />
        <div {...tabPanelProps(id, view)}>
          {view === "raw" ? (
            <CodeView text={JSON.stringify(evidence, null, 2)} label="Raw evidence JSON" wrap />
          ) : evidence.tool === "shell" ? (
            <Shell evidence={evidence} />
          ) : evidence.tool === "http" ? (
            <Http evidence={evidence} />
          ) : evidence.tool === "mcp" ? (
            <Mcp evidence={evidence} />
          ) : (
            <Rejected evidence={evidence} />
          )}
        </div>
      </div>
    </section>
  );
}
