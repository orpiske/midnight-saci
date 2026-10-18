import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { SafeText, segments } from "./text";

describe("SafeText", () => {
  it("never renders HTML from text", () => {
    const { container } = render(
      <pre>
        <SafeText text={'<img src=x onerror="alert(1)"><script>alert(2)</script>'} />
      </pre>,
    );
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("script")).toBeNull();
    expect(container.textContent).toContain("<script>");
  });

  it("shows terminal escapes and bidi overrides instead of interpreting them", () => {
    const parts = segments("\u001b[31mred\u001b[0m ‮evil");
    expect(parts.map((p) => p.value).join("")).toBe("␛[31mred␛[0m <U+202E>evil");
    expect(parts.filter((p) => p.kind === "control")).toHaveLength(3);
  });

  it("keeps tabs and newlines and highlights redaction markers", () => {
    const parts = segments("token=[REDACTED:API_TOKEN]\n\tnext [REDACTED]");
    expect(parts).toEqual([
      { kind: "text", value: "token=" },
      { kind: "redacted", value: "[REDACTED:API_TOKEN]" },
      { kind: "text", value: "\n\tnext " },
      { kind: "redacted", value: "[REDACTED]" },
    ]);
  });
});
