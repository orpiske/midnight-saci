import { Check, Copy } from "lucide-react";
import { useEffect, useState } from "react";

type CopyState = "idle" | "copied" | "failed";

const MESSAGE: Record<CopyState, string> = { idle: "", copied: "Copied", failed: "Copy failed" };

/** How long the copy result stays visible. */
export const COPY_FEEDBACK_MS = 1500;

/** Copies `value` to the clipboard; shows `label` (shortened text). */
export function CopyText({ value, label, title }: { value: string; label?: string; title?: string }) {
  const [state, setState] = useState<CopyState>("idle");
  useEffect(() => {
    if (state === "idle") return;
    const timer = window.setTimeout(() => setState("idle"), COPY_FEEDBACK_MS);
    return () => window.clearTimeout(timer);
  }, [state]);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setState("copied");
    } catch {
      // No clipboard access, for example a page served over plain HTTP.
      setState("failed");
    }
  };
  return (
    <span className="row nowrap">
      <span className="mono num" title={value}>
        {label ?? value}
      </span>
      <button
        type="button"
        className="icon-btn"
        onClick={() => void copy()}
        aria-label={`Copy ${title ?? value}`}
        title="Copy"
      >
        {state === "copied" ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
      </button>
      {/* Always rendered so screen readers hear the change. */}
      <span className={`copy-feedback copy-${state}`} role="status" aria-live="polite">
        {MESSAGE[state]}
      </span>
    </span>
  );
}
