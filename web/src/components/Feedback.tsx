import { CircleAlert, Info, LoaderCircle, TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";

import { ApiError, NetworkError } from "../api/client";

export function Loading({ label = "Loading…" }: { label?: string }) {
  return (
    <div className="loading" role="status">
      <LoaderCircle size={16} className="spin" aria-hidden="true" />
      <span>{label}</span>
    </div>
  );
}

type Tone = "fail" | "warn" | "run" | "neutral";

export function Callout({
  tone,
  title,
  children,
  role,
}: {
  tone: Tone;
  title?: ReactNode;
  children?: ReactNode;
  role?: "alert" | "status";
}) {
  const Icon = tone === "fail" ? CircleAlert : tone === "warn" ? TriangleAlert : Info;
  return (
    <div className={`callout tone-${tone}`} role={role}>
      <Icon size={16} aria-hidden="true" />
      <div className="stack-sm">
        {title ? <div className="callout-title">{title}</div> : null}
        {children ? <div>{children}</div> : null}
      </div>
    </div>
  );
}

/** Plain, actionable text for an error (artwork/BRAND.md section 8). */
export function describeError(error: unknown): string {
  if (error instanceof NetworkError) {
    return "Couldn't reach the backend. Check that saci-backend is running, then try again.";
  }
  if (error instanceof ApiError) {
    if (error.status === 401) return "The backend needs a client token. Connect again with it.";
    if (error.status === 404) return "Not found. It may have been removed, or the link is wrong.";
    return error.message;
  }
  return "Something went wrong in the console. Reload the page to try again.";
}

export function ErrorNotice({
  error,
  title = "This could not be loaded",
  retry,
}: {
  error: unknown;
  title?: string;
  retry?: () => void;
}) {
  return (
    <Callout tone="fail" title={title} role="alert">
      <div className="stack-sm">
        <span>{describeError(error)}</span>
        {retry ? (
          <span>
            <button type="button" className="btn btn-sm" onClick={retry}>
              Try again
            </button>
          </span>
        ) : null}
      </div>
    </Callout>
  );
}

export function EmptyState({
  title,
  children,
  action,
}: {
  title: string;
  children?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <p className="display">{title}</p>
      {children ? <p className="muted">{children}</p> : null}
      {action}
    </div>
  );
}
