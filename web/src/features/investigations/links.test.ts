import { describe, expect, it } from "vitest";

import type { CodeLocation, Repository } from "../../api/types";
import { codeLink, httpsLink, lineRange } from "./links";

const COMMIT = "0123456789abcdef0123456789abcdef01234567";

const code = (repository: string, extra: Partial<CodeLocation> = {}): CodeLocation => ({
  repository,
  commit: COMMIT,
  path: "src/health check.rs",
  start_line: 10,
  end_line: 14,
  ...extra,
});

const repositories: Repository[] = [
  { name: "orders", url: "https://github.com/example/orders.git", ref: "main" },
  { name: "billing", url: "git@gitlab.com:example/billing.git", ref: "main" },
  { name: "local", url: "https://git.example.com/orders.git", ref: "main" },
  { name: "plain", url: "http://github.com/example/plain.git", ref: "main" },
];

describe("codeLink", () => {
  it("points at the cited lines at the cited commit", () => {
    expect(codeLink(code("orders"), repositories)).toBe(
      `https://github.com/example/orders/blob/${COMMIT}/src/health%20check.rs#L10-L14`,
    );
    expect(codeLink(code("billing", { end_line: 10 }), repositories)).toBe(
      `https://gitlab.com/example/billing/-/blob/${COMMIT}/src/health%20check.rs#L10`,
    );
  });

  it("gives no link for an unknown forge, plain HTTP, an unknown repository or a bad commit", () => {
    expect(codeLink(code("local"), repositories)).toBeNull();
    expect(codeLink(code("plain"), repositories)).toBeNull();
    expect(codeLink(code("missing"), repositories)).toBeNull();
    expect(codeLink(code("orders", { commit: "main/../x" }), repositories)).toBeNull();
  });
});

describe("links", () => {
  it("keeps only https bug links", () => {
    expect(httpsLink("https://github.com/example/orders/issues/7")).toBe("https://github.com/example/orders/issues/7");
    expect(httpsLink("javascript:alert(1)")).toBeNull();
    expect(httpsLink("not a url")).toBeNull();
  });

  it("writes one line without a range", () => {
    expect(lineRange(code("orders", { end_line: 10 }))).toBe("10");
    expect(lineRange(code("orders"))).toBe("10-14");
  });
});
