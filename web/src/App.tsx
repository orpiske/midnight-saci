import { Inbox, ListChecks, Menu, PlugZap, X } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";

import { Wordmark } from "./components/Brand";
import { EmptyState } from "./components/Feedback";
import { ConnectionPage, TokenForm } from "./features/connection/ConnectionPage";
import { connectionSummary, useConnection } from "./features/connection/connection";
import { RunsPage } from "./features/runs/RunsPage";
import { NewRunPage } from "./features/submission/NewRunPage";
import { Link, navigate, useLocation } from "./lib/router";

type Section = "runs" | "actions" | "connection";

function NavLink({
  to,
  section,
  current,
  icon,
  children,
  onNavigate,
}: {
  to: string;
  section: Section;
  current: Section | null;
  icon: ReactNode;
  children: ReactNode;
  onNavigate: () => void;
}) {
  return (
    <li>
      <Link
        to={to}
        className="nav-item"
        aria-current={current === section ? "page" : undefined}
        onClick={onNavigate}
      >
        {icon}
        {children}
      </Link>
    </li>
  );
}

function Navigation({
  current,
  open,
  close,
}: {
  current: Section | null;
  open: boolean;
  close: () => void;
}) {
  const connection = useConnection();
  const summary = connectionSummary(connection);
  return (
    <nav className={`nav${open ? " open" : ""}`} aria-label="Main" id="main-nav">
      <Link to="/runs" className="nav-brand" aria-label="Midnight Saci, runs" onClick={close}>
        <Wordmark />
      </Link>
      <ul className="nav-list">
        <NavLink
          to="/runs"
          section="runs"
          current={current}
          icon={<ListChecks size={16} aria-hidden="true" />}
          onNavigate={close}
        >
          Runs
        </NavLink>
        <NavLink
          to="/actions"
          section="actions"
          current={current}
          icon={<Inbox size={16} aria-hidden="true" />}
          onNavigate={close}
        >
          Outcome actions
        </NavLink>
      </ul>
      <ul className="nav-list nav-footer">
        <li>
          <Link
            to="/connection"
            className="nav-item nav-connection"
            aria-current={current === "connection" ? "page" : undefined}
            onClick={close}
          >
            <PlugZap size={16} aria-hidden="true" style={{ marginTop: 3 }} />
            <span>
              Connection
              <small>
                <span className={`dot tone-${summary.tone}`} aria-hidden="true" />
                {summary.label}
              </small>
            </span>
          </Link>
        </li>
      </ul>
    </nav>
  );
}

function Screen({ path }: { path: string }): ReactNode {
  if (path === "/" || path === "/runs") return <RunsPage />;
  if (path === "/runs/new") return <NewRunPage />;
  if (path === "/connection") return <ConnectionPage />;
  return (
    <div className="page">
      <EmptyState
        title="Nothing lives here"
        action={
          <Link to="/runs" className="btn">
            Go to runs
          </Link>
        }
      >
        This address is not part of the console.
      </EmptyState>
    </div>
  );
}

/** Shown instead of a screen while the backend needs a token. */
function TokenGate() {
  const { auth, connect } = useConnection();
  return (
    <div className="page narrow stack">
      <h1>Connect to the backend</h1>
      <TokenForm rejected={auth === "rejected"} connect={connect} />
    </div>
  );
}

function sectionOf(path: string): Section | null {
  if (path === "/" || path.startsWith("/runs")) return "runs";
  if (path.startsWith("/actions")) return "actions";
  if (path.startsWith("/connection")) return "connection";
  return null;
}

export function App() {
  const { path } = useLocation();
  const connection = useConnection();
  const [menu, setMenu] = useState(false);
  const section = sectionOf(path);

  useEffect(() => {
    if (path === "/") navigate("/runs", { replace: true });
  }, [path]);

  useEffect(() => {
    const titles: Record<Section, string> = {
      runs: "Runs",
      actions: "Outcome actions",
      connection: "Connection",
    };
    document.title = section ? `${titles[section]} · Midnight Saci` : "Midnight Saci";
  }, [section]);

  const gated =
    section !== "connection" && (connection.auth === "needed" || connection.auth === "rejected");

  return (
    <div className="app">
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <header className="topbar">
        <Link to="/runs" aria-label="Midnight Saci, runs">
          <Wordmark />
        </Link>
        <button
          type="button"
          className="icon-btn"
          aria-expanded={menu}
          aria-controls="main-nav"
          aria-label={menu ? "Close navigation" : "Open navigation"}
          onClick={() => setMenu((open) => !open)}
        >
          {menu ? <X size={18} aria-hidden="true" /> : <Menu size={18} aria-hidden="true" />}
        </button>
      </header>
      <Navigation current={section} open={menu} close={() => setMenu(false)} />
      <main id="main" className="main" tabIndex={-1}>
        {gated ? <TokenGate /> : <Screen key={connection.generation} path={path} />}
      </main>
    </div>
  );
}
