import { type ReactNode, useEffect, useRef } from "react";

import { SafeText } from "../lib/text";

/** Text with line numbers in Fog on the darker log background. Lines in
 * `diagnostics` are marked; `highlight` is scrolled into view. */
export function CodeView({
  text,
  label,
  highlight,
  diagnostics,
  wrap = false,
  emptyLabel = "Empty",
  maxHeight,
}: {
  text: string;
  label: string;
  highlight?: number | null;
  diagnostics?: ReadonlySet<number>;
  wrap?: boolean;
  emptyLabel?: string;
  maxHeight?: number;
}) {
  const container = useRef<HTMLPreElement>(null);
  useEffect(() => {
    if (!highlight || !container.current) return;
    const line = container.current.querySelector(`[data-line="${highlight}"]`);
    if (line instanceof HTMLElement) {
      const top = line.offsetTop - container.current.clientHeight / 3;
      container.current.scrollTop = Math.max(0, top);
    }
  }, [highlight, text]);

  const lines = text.length === 0 ? [] : text.replace(/\n$/, "").split("\n");
  let body: ReactNode;
  if (lines.length === 0) {
    body = <div className="code-empty">{emptyLabel}</div>;
  } else {
    body = (
      <div className="code-lines">
        {lines.map((line, index) => {
          const number = index + 1;
          const marks = [
            number === highlight ? "highlight" : "",
            diagnostics?.has(number) ? "diagnostic" : "",
          ].join(" ");
          return (
            <div key={number} className={`code-line ${marks}`} data-line={number}>
              <span className="ln" aria-hidden="true">
                {number}
              </span>
              <span className="lc">
                <SafeText text={line} />
              </span>
            </div>
          );
        })}
      </div>
    );
  }
  return (
    <pre
      ref={container}
      className={`code${wrap ? " code-wrap" : ""}`}
      aria-label={label}
      tabIndex={0}
      style={maxHeight ? { maxHeight } : undefined}
    >
      {body}
    </pre>
  );
}
