import { useId } from "react";

import type { EnvironmentInfo, PolicyInfo } from "../../api/types";
import { Callout } from "../../components/Feedback";
import { formatBytes, formatSeconds, plural } from "../../lib/format";
import { OUTCOME } from "../../lib/status";

/** Environment and policy choices from the backend's configuration, with
 * what they mean for the run. Credentials stay on the backend. */
export function RunSettings({
  environments,
  policies,
  environment,
  policy,
  disabled,
  onEnvironment,
  onPolicy,
}: {
  environments: EnvironmentInfo[];
  policies: PolicyInfo[];
  environment: string;
  policy: string;
  disabled: boolean;
  onEnvironment: (name: string) => void;
  onPolicy: (name: string) => void;
}) {
  const id = useId();
  if (environments.length === 0) {
    return (
      <Callout tone="warn" title="No environments are configured">
        Runs need a target environment. Add one under <code className="inline-code">environments</code>{" "}
        in the backend configuration and restart it.
      </Callout>
    );
  }
  const env = environments.find((e) => e.name === environment);
  const pol = policies.find((p) => p.name === policy);
  return (
    <div className="stack">
      <div className="form-grid">
        <div className="field">
          <label htmlFor={`${id}-env`}>Environment</label>
          <select
            id={`${id}-env`}
            value={environment}
            onChange={(event) => onEnvironment(event.target.value)}
            disabled={disabled}
          >
            {environments.map((e) => (
              <option key={e.name} value={e.name}>
                {e.name}
              </option>
            ))}
          </select>
          {env?.description ? <span className="muted small">{env.description}</span> : null}
        </div>
        <div className="field">
          <label htmlFor={`${id}-policy`}>Policy</label>
          <select
            id={`${id}-policy`}
            value={policy}
            onChange={(event) => onPolicy(event.target.value)}
            disabled={disabled}
          >
            {policies.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name}
                {p.configured ? "" : " (built-in)"}
              </option>
            ))}
          </select>
        </div>
      </div>
      <div className="form-grid">
        {env ? <LimitsSummary env={env} /> : null}
        {pol ? <PolicySummary policy={pol} /> : null}
      </div>
    </div>
  );
}

function LimitsSummary({ env }: { env: EnvironmentInfo }) {
  const l = env.limits;
  return (
    <div className="stack-sm">
      <h3>Time limits</h3>
      <dl className="kv num">
        <dt>Whole run</dt>
        <dd>{formatSeconds(l.run_timeout_secs)}</dd>
        <dt>Each attempt</dt>
        <dd>{formatSeconds(l.step_timeout_secs)}</dd>
        <dt>Each command or call</dt>
        <dd>{formatSeconds(l.operation_timeout_secs)}</dd>
        <dt>Per attempt</dt>
        <dd>
          {plural(l.model_calls_per_step, "model request")}, {plural(l.tool_calls_per_step, "tool call")}
        </dd>
        <dt>Output kept</dt>
        <dd>{formatBytes(l.output_limit_bytes)} per stream</dd>
        {env.mcp_servers?.length ? (
          <>
            <dt>MCP servers</dt>
            <dd>{env.mcp_servers.join(", ")}</dd>
          </>
        ) : null}
      </dl>
    </div>
  );
}

function PolicySummary({ policy }: { policy: PolicyInfo }) {
  const retryOn = policy.retry.on ?? [];
  const actions = policy.actions ?? [];
  return (
    <div className="stack-sm">
      <h3>On failure</h3>
      <dl className="kv">
        <dt>Unsuccessful unit</dt>
        <dd>
          {policy.on_failure === "stop"
            ? "Stop; later units are skipped"
            : "Record it and continue with the next unit"}
        </dd>
        <dt>Attempts per unit</dt>
        <dd>
          {policy.retry.max_attempts === 1 || retryOn.length === 0
            ? "1, no retries"
            : `Up to ${policy.retry.max_attempts}, retried when ${retryOn
                .map((o) => OUTCOME[o].label.toLowerCase())
                .join(" or ")}`}
        </dd>
        <dt>Follow-up actions</dt>
        <dd>
          {actions.length === 0 ? (
            "None"
          ) : (
            <ul className="stack-sm" style={{ margin: 0, paddingLeft: 18 }}>
              {actions.map((a) => (
                <li key={a.name}>
                  <span className="mono">{a.name}</span>: {a.server} / {a.tool} when the run is{" "}
                  {a.on.map((o) => OUTCOME[o].label.toLowerCase()).join(" or ")}
                  {a.approval === "required" ? ", after approval" : ""}
                </li>
              ))}
            </ul>
          )}
        </dd>
      </dl>
    </div>
  );
}
