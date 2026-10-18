import { describe, expect, it } from "vitest";

import { ApiError, type Client, NetworkError } from "../../api/client";
import type { SubmitRun } from "../../api/types";
import { fileProblems, guessEntry, pathProblem, payloadText } from "./bundle";
import { keyFor, send } from "./submit";

const request: SubmitRun = {
  bundle: { entry: "plan.md", documents: { "plan.md": "# Test Plan: A\n" } },
  environment: "local",
  policy: "default",
};

function recordingClient(answers: (() => Promise<{ status: number; body: unknown }>)[]) {
  const keys: string[] = [];
  const client: Client = {
    get: () => Promise.reject(new Error("unused")),
    post: async <T>(_path: string, _body: unknown, options?: { headers?: Record<string, string> }) => {
      keys.push(options?.headers?.["Idempotency-Key"] ?? "");
      const next = answers.shift();
      if (!next) throw new Error("unexpected request");
      return (await next()) as { status: number; body: T };
    },
  };
  return { client, keys };
}

describe("retry-safe submission", () => {
  it("reuses the key after a lost answer and creates one run", async () => {
    const run = { id: "run-abc" };
    const { client, keys } = recordingClient([
      () => Promise.reject(new NetworkError("lost")),
      () => Promise.resolve({ status: 200, body: run }),
    ]);
    let pending = keyFor(null, payloadText(request));
    const first = await send(client, request, pending);
    expect(first.kind).toBe("unresolved");
    pending = keyFor(pending, payloadText(request));
    const retry = await send(client, request, pending);
    expect(retry).toMatchObject({ kind: "accepted", replayed: true });
    expect(keys).toHaveLength(2);
    expect(keys[0]).toBe(keys[1]);
  });

  it("gives an edited payload a new key", () => {
    const pending = keyFor(null, payloadText(request));
    const edited = { ...request, environment: "staging" };
    expect(keyFor(pending, payloadText(edited)).key).not.toBe(pending.key);
    expect(keyFor(pending, payloadText({ ...request })).key).toBe(pending.key);
  });

  it("treats a server error as unresolved and a 422 as invalid", async () => {
    const { client } = recordingClient([
      () => Promise.reject(new ApiError(500, "internal", "database")),
      () =>
        Promise.reject(
          new ApiError(422, "validation_failed", "the plan is invalid", ["plan.md:3: x"], [
            { document: "plan.md", line: 3, message: "x" },
          ]),
        ),
    ]);
    const pending = keyFor(null, "p");
    expect((await send(client, request, pending)).kind).toBe("unresolved");
    const invalid = await send(client, request, pending);
    expect(invalid).toMatchObject({ kind: "invalid", diagnostics: [{ line: 3 }] });
  });

  it("orders documents so equal bundles give equal payloads", () => {
    const a = { ...request, bundle: { entry: "p.md", documents: { "b.md": "b", "p.md": "p" } } };
    const b = { ...request, bundle: { entry: "p.md", documents: { "p.md": "p", "b.md": "b" } } };
    expect(payloadText(a)).toBe(payloadText(b));
  });
});

describe("bundle paths", () => {
  it("rejects unsafe paths like the backend does", () => {
    expect(pathProblem("plan.md")).toBeNull();
    expect(pathProblem("common/setup.md")).toBeNull();
    for (const bad of ["", "/etc/passwd", "../x.md", "a/./b.md", "a\\b.md", "a//b.md", "a/"]) {
      expect(pathProblem(bad), bad).not.toBeNull();
    }
  });

  it("flags duplicates on both files", () => {
    const files = [
      { id: "1", path: "a.md", text: "", size: 0 },
      { id: "2", path: "a.md", text: "", size: 0 },
      { id: "3", path: "b.md", text: "", size: 0 },
    ];
    expect([...fileProblems(files).keys()].sort()).toEqual(["1", "2"]);
  });

  it("guesses the shallowest test plan as the entry", () => {
    const files = [
      { id: "p", path: "common/setup.md", text: "# Procedure: Setup", size: 0 },
      { id: "n", path: "nested/plan.md", text: "# Test Plan: Nested", size: 0 },
      { id: "e", path: "plan.md", text: "# Test Plan: Entry", size: 0 },
    ];
    expect(guessEntry(files)).toBe("e");
  });
});
