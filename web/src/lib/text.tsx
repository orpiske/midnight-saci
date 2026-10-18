// Faithful, inert rendering of untrusted text: tool output, Markdown
// sources, action results and API messages. Text is always rendered as
// React text (escaped), never as HTML. Terminal escapes and other control
// characters are shown as visible symbols, never interpreted, and Saci's
// redaction markers are highlighted.
import type { ReactNode } from "react";

const REDACTION = /\[REDACTED(?::[A-Za-z0-9_]+)?\]/g;

// C0 controls except tab and newline, DEL, C1 controls, and the bidi
// overrides and isolates that can reorder what a reader sees.
const CONTROL = /[\u0000-\u0008\u000B-\u001F\u007F-\u009F‪-‮⁦-⁩]/g;

/** The visible stand-in for one control character. */
export function controlPicture(char: string): string {
  const code = char.charCodeAt(0);
  if (code <= 0x1f) return String.fromCharCode(0x2400 + code);
  if (code === 0x7f) return "␡";
  return `<U+${code.toString(16).toUpperCase().padStart(4, "0")}>`;
}

/** Splits `text` into plain, control and redaction segments. */
export function segments(text: string): { kind: "text" | "control" | "redacted"; value: string }[] {
  const out: { kind: "text" | "control" | "redacted"; value: string }[] = [];
  const pattern = new RegExp(`${REDACTION.source}|${CONTROL.source}`, "g");
  let last = 0;
  for (const found of text.matchAll(pattern)) {
    const index = found.index ?? 0;
    if (index > last) out.push({ kind: "text", value: text.slice(last, index) });
    const value = found[0];
    if (value.startsWith("[REDACTED")) out.push({ kind: "redacted", value });
    else out.push({ kind: "control", value: controlPicture(value) });
    last = index + value.length;
  }
  if (last < text.length) out.push({ kind: "text", value: text.slice(last) });
  return out;
}

/** Inert text with visible control characters and highlighted redactions. */
export function SafeText({ text }: { text: string }): ReactNode {
  return segments(text).map((part, i) => {
    if (part.kind === "redacted") {
      return (
        <mark key={i} className="redacted" title="Redacted by Saci before it was stored">
          {part.value}
        </mark>
      );
    }
    if (part.kind === "control") {
      return (
        <span key={i} className="ctrl" title="Control character, shown and not interpreted">
          {part.value}
        </span>
      );
    }
    return part.value;
  });
}
