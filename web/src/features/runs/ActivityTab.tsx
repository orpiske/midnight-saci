// The live event feed. Only the newest events are rendered, so a long run
// stays responsive; the view follows the newest event until the reader
// scrolls away or pauses it.
import { Pause, Play } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";

import type { AttemptTiming, RunEvent } from "../../api/types";
import { Callout, describeError } from "../../components/Feedback";
import { StatusIcon } from "../../components/Status";
import { formatClock } from "../../lib/format";
import { setSearch } from "../../lib/router";
import { IN_PROGRESS, OUTCOME, PENDING, type StatusLook } from "../../lib/status";
import { SafeText } from "../../lib/text";
import { TOOL_LABEL, evidenceSummary } from "../evidence/EvidenceView";
import type { Activity } from "./RunPage";

/** Events rendered at once. */
export const RENDERED_EVENTS = 500;

/** Where a finished attempt spent its time; empty when the worker did not say. */
function timingNote(timing?: AttemptTiming): string {
  if (!timing) return "";
  const requests = timing.model_requests === 1 ? "request" : "requests";
  return ` (model: ${timing.model_requests} ${requests}, ${timing.model_ms} ms; tools: ${timing.tool_ms} ms)`;
}

/** What one event says, in plain words. */
export function describeEvent(event: RunEvent): { look?: StatusLook; text: ReactNode } {
  switch (event.type) {
    case "run_started":
      return { text: `Run started with model ${event.model}` };
    case "attempt_started":
      return { look: IN_PROGRESS, text: `Attempt ${event.attempt ?? ""} started` };
    case "observation":
      return {
        text: (
          <>
            {TOOL_LABEL[event.evidence.tool]}: <SafeText text={evidenceSummary(event.evidence)} />
          </>
        ),
      };
    case "attempt_finished": {
      const outcome = OUTCOME[event.outcome].label.toLowerCase();
      const error = event.error ? `: ${event.error.message}` : "";
      return {
        look: OUTCOME[event.outcome],
        text: `Attempt ${event.attempt ?? ""} ${outcome}${timingNote(event.timing)}${error}`,
      };
    }
    case "unit_finished":
      return { look: OUTCOME[event.outcome], text: `Unit ${OUTCOME[event.outcome].label.toLowerCase()}` };
    case "unit_skipped":
      return { look: OUTCOME.skipped, text: <>Skipped: <SafeText text={event.reason} /></> };
    case "cancellation_requested":
      return { look: PENDING, text: "Cancellation requested; stopping" };
    case "run_finished":
      return {
        look: OUTCOME[event.outcome],
        text: (
          <>
            Run {OUTCOME[event.outcome].label.toLowerCase()}: <SafeText text={event.reason} />
          </>
        ),
      };
  }
}

const STATUS_TEXT = {
  live: "Live",
  reconnecting: "Reconnecting",
  done: "Finished, no more events",
  stopped: "Stopped following",
} as const;

export function ActivityTab({ activity }: { activity: Activity }) {
  const [following, setFollowing] = useState(true);
  const list = useRef<HTMLDivElement>(null);
  const shown = activity.events.slice(-RENDERED_EVENTS);
  const hidden = activity.total - shown.length;

  useEffect(() => {
    const element = list.current;
    if (following && element) element.scrollTop = element.scrollHeight;
  }, [following, activity.events]);

  const onScroll = () => {
    const element = list.current;
    if (!element) return;
    const atBottom = element.scrollHeight - element.scrollTop - element.clientHeight < 24;
    if (!atBottom && following) setFollowing(false);
  };

  const { status, lastUpdate, error } = activity.state;
  return (
    <section className="panel" aria-label="Activity">
      <div className="activity-bar">
        <span className="row small">
          <span className={`dot tone-${status === "live" ? "run" : status === "reconnecting" ? "warn" : "neutral"}`} aria-hidden="true" />
          <span>{STATUS_TEXT[status]}</span>
          <span className="muted num">Last update {formatClock(lastUpdate)}</span>
        </span>
        <button
          type="button"
          className="btn btn-sm"
          aria-pressed={following}
          onClick={() => setFollowing((value) => !value)}
        >
          {following ? <Pause size={14} aria-hidden="true" /> : <Play size={14} aria-hidden="true" />}
          {following ? "Pause following" : "Follow newest"}
        </button>
      </div>
      {status === "stopped" ? (
        <div className="panel-body">
          <Callout tone="fail" title="Progress is no longer followed" role="alert">
            {describeError(error)} Refresh the page to try again.
          </Callout>
        </div>
      ) : null}
      {hidden > 0 ? (
        <p className="muted small" style={{ padding: "8px 16px 0" }}>
          Showing the newest {shown.length} of {activity.total} events. The report on the Results
          tab holds everything.
        </p>
      ) : null}
      <div className="activity" ref={list} onScroll={onScroll} tabIndex={0} aria-label="Run events">
        {shown.length === 0 ? (
          <p className="muted" style={{ padding: 16 }}>
            No events yet.
          </p>
        ) : (
          <ol className="plain-list">
            {shown.map((event) => (
              <EventRow key={event.seq} event={event} />
            ))}
          </ol>
        )}
      </div>
    </section>
  );
}

function EventRow({ event }: { event: RunEvent }) {
  const { look, text } = describeEvent(event);
  const open =
    event.type === "observation"
      ? () =>
          setSearch(
            {
              tab: null,
              unit: event.evidence.unit,
              attempt: String(event.evidence.attempt),
              evidence: event.evidence.id,
            },
            true,
          )
      : null;
  return (
    <li className="event-row">
      <span className="seq">#{event.seq}</span>
      <time dateTime={event.at}>{formatClock(event.at)}</time>
      <span className="unit mono small">{event.unit ?? "run"}</span>
      <span className="what row">
        {look ? <StatusIcon look={look} size={14} /> : null}
        <span>{text}</span>
        {open ? (
          <button type="button" className="link-btn small" onClick={open}>
            View evidence
          </button>
        ) : null}
      </span>
    </li>
  );
}
