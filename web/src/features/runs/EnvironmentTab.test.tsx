import { describe, expect, it } from "vitest";

import { buildsDiffer, shortCommit } from "./EnvironmentTab";

const build = (commit?: string) => ({ version: "0.1.0", ...(commit ? { commit } : {}) });

describe("buildsDiffer", () => {
  it("warns only when both commits are known and differ", () => {
    expect(buildsDiffer(build("aaa"), build("bbb"))).toBe(true);
    expect(buildsDiffer(build("aaa"), build("aaa"))).toBe(false);
    expect(buildsDiffer(build("aaa"), build())).toBe(false);
    expect(buildsDiffer(undefined, build("bbb"))).toBe(false);
  });
});

describe("shortCommit", () => {
  it("keeps the first twelve characters of a full hash", () => {
    expect(shortCommit("0123456789abcdef0123456789abcdef01234567")).toBe("0123456789ab");
    expect(shortCommit("abc")).toBe("abc");
  });
});
