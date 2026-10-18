import { Check, Copy } from "lucide-react";
import { useState } from "react";

/** Copies `value` to the clipboard; shows `label` (shortened text). */
export function CopyText({ value, label, title }: { value: string; label?: string; title?: string }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      setCopied(false);
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
        aria-label={copied ? "Copied" : `Copy ${title ?? value}`}
        title={copied ? "Copied" : "Copy"}
      >
        {copied ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
      </button>
    </span>
  );
}
