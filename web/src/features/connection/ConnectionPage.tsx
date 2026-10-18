import { KeyRound, LogOut, RefreshCw } from "lucide-react";
import { type FormEvent, useId, useState } from "react";

import { Lockup } from "../../components/Brand";
import { Callout } from "../../components/Feedback";
import { formatClock } from "../../lib/format";
import { useConnection } from "./connection";

export function ConnectionPage() {
  const connection = useConnection();
  return (
    <div className="page narrow">
      <Lockup />
      <div className="stack">
        <h1>Connection</h1>
        <BackendCard />
        {connection.health === "reachable" ? <AuthCard /> : null}
        <p className="muted small">
          Health means this backend answers. It does not check targets, workers or the model
          provider; a run reports those problems itself.
        </p>
      </div>
    </div>
  );
}

function BackendCard() {
  const { health, lastSeen, recheck } = useConnection();
  const [checking, setChecking] = useState(false);
  const check = async () => {
    setChecking(true);
    await recheck();
    setChecking(false);
  };
  return (
    <section className="panel" aria-labelledby="backend-heading">
      <div className="panel-header">
        <h2 id="backend-heading">Backend</h2>
        <button type="button" className="btn btn-sm" onClick={() => void check()} disabled={checking}>
          <RefreshCw size={14} aria-hidden="true" className={checking ? "spin" : undefined} />
          Check again
        </button>
      </div>
      <div className="panel-body stack">
        <dl className="kv">
          <dt>Address</dt>
          <dd className="mono">{window.location.origin}</dd>
          <dt>Status</dt>
          <dd>
            {health === "checking"
              ? "Checking…"
              : health === "reachable"
                ? "Reachable"
                : "Unreachable"}
          </dd>
          <dt>Last answer</dt>
          <dd className="num">{lastSeen ? formatClock(lastSeen) : "—"}</dd>
        </dl>
        {health === "unreachable" ? (
          <Callout tone="fail" title="Couldn't reach the backend" role="alert">
            Check that saci-backend is running and that this page is served by it or by the
            development server's proxy. The console checks again every 10 seconds.
          </Callout>
        ) : null}
      </div>
    </section>
  );
}

function AuthCard() {
  const { auth, connect, disconnect } = useConnection();
  return (
    <section className="panel" aria-labelledby="auth-heading">
      <div className="panel-header">
        <h2 id="auth-heading">Client authentication</h2>
      </div>
      <div className="panel-body stack">
        {auth === "open" ? (
          <p>This backend listens on loopback and needs no client token.</p>
        ) : null}
        {auth === "token" ? (
          <>
            <p>Connected with a client token. It is kept in this tab's memory only and is gone when you close or reload the tab.</p>
            <span>
              <button type="button" className="btn" onClick={disconnect}>
                <LogOut size={16} aria-hidden="true" />
                Disconnect
              </button>
            </span>
          </>
        ) : null}
        {auth === "needed" || auth === "rejected" || auth === "unknown" ? (
          <TokenForm rejected={auth === "rejected"} connect={connect} />
        ) : null}
      </div>
    </section>
  );
}

export function TokenForm({
  rejected,
  connect,
}: {
  rejected: boolean;
  connect: (token: string) => Promise<boolean>;
}) {
  const id = useId();
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!value.trim() || busy) return;
    setBusy(true);
    const ok = await connect(value);
    setBusy(false);
    if (ok) setValue("");
  };
  return (
    <form className="stack" onSubmit={(event) => void submit(event)}>
      <p>
        This backend requires a client token: the value of the environment variable named by{" "}
        <code className="inline-code">backend.auth.token_env</code>. It stays in this tab's memory
        and is never stored or put in a link.
      </p>
      <div className="field">
        <label htmlFor={id}>Client token</label>
        <input
          id={id}
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={value}
          onChange={(event) => setValue(event.target.value)}
          aria-invalid={rejected}
          aria-describedby={rejected ? `${id}-error` : undefined}
        />
        {rejected ? (
          <span id={`${id}-error`} className="field-error" role="alert">
            The backend refused this token. Check the value and try again.
          </span>
        ) : null}
      </div>
      <span>
        <button type="submit" className="btn btn-primary" disabled={busy || !value.trim()}>
          <KeyRound size={16} aria-hidden="true" />
          {busy ? "Connecting…" : "Connect"}
        </button>
      </span>
    </form>
  );
}
