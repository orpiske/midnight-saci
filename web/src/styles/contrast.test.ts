// WCAG 2.2 contrast of the color pairs the console uses, computed from
// tokens.css itself so a token change cannot silently break legibility.
// @vitest-environment node
import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

const css = readFileSync(new URL("./tokens.css", import.meta.url), "utf8");

function token(name: string): string {
  const found = new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})`).exec(css);
  if (!found?.[1]) throw new Error(`token --${name} not found`);
  return found[1];
}

type Rgb = [number, number, number];

function rgb(hex: string): Rgb {
  const n = Number.parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** `share` of `top` over `bottom`, as CSS `color-mix(in srgb, …)` does. */
function mix(top: Rgb, share: number, bottom: Rgb): Rgb {
  return top.map((c, i) => c * share + (bottom[i] ?? 0) * (1 - share)) as Rgb;
}

function luminance([r, g, b]: Rgb): number {
  const channel = (value: number) => {
    const c = value / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrast(a: Rgb, b: Rgb): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (light + 0.05) / (dark + 0.05);
}

const TEXT = 4.5;
const LARGE_OR_UI = 3;
const SURFACES = ["ms-bg", "ms-surface", "ms-raised", "ms-code-bg"];
const TINT = Number(/--tint:\s*(\d+)%/.exec(css)?.[1] ?? Number.NaN) / 100;

describe("design token contrast", () => {
  it.each(SURFACES.flatMap((s) => ["ms-text", "ms-text-muted"].map((t) => [t, s])))(
    "%s on %s is readable body text",
    (text, surface) => {
      expect(contrast(rgb(token(text)), rgb(token(surface)))).toBeGreaterThanOrEqual(TEXT);
    },
  );

  it.each([["ms-accent-fill"], ["ms-accent-fill-hover"]])("white labels on %s", (fill) => {
    expect(contrast(rgb(token("ms-on-accent")), rgb(token(fill)))).toBeGreaterThanOrEqual(TEXT);
  });

  it("documents why the brand red is not a button fill", () => {
    expect(contrast(rgb(token("ms-on-accent")), rgb(token("ms-accent")))).toBeLessThan(TEXT);
  });

  const statuses: [string, string][] = [
    ["ms-pass", "ms-pass"],
    ["ms-fail", "ms-fail"],
    ["ms-flaky", "ms-flaky"],
    ["ms-running", "ms-running"],
    ["ms-neutral-text", "ms-skipped"],
  ];
  it.each(statuses.flatMap(([text, tone]) => ["ms-surface", "ms-raised", "ms-bg"].map((s) => [text, tone, s])))(
    "badge text %s on its %s tint over %s",
    (text, tone, surface) => {
      const background = mix(rgb(token(tone)), TINT, rgb(token(surface)));
      expect(contrast(rgb(token(text)), background)).toBeGreaterThanOrEqual(TEXT);
    },
  );

  it.each([
    ["ms-glow", "ms-raised"],
    ["ms-glow", "ms-code-bg"],
    ["ms-fail", "ms-surface"],
  ])("marker text %s on %s", (text, surface) => {
    expect(contrast(rgb(token(text)), rgb(token(surface)))).toBeGreaterThanOrEqual(TEXT);
  });

  it.each([
    ["ms-text-faint", "ms-bg"],
    ["ms-glow", "ms-bg"],
    ["ms-accent", "ms-surface"],
  ])("non-text %s on %s (placeholders, focus ring, active marker)", (color, surface) => {
    expect(contrast(rgb(token(color)), rgb(token(surface)))).toBeGreaterThanOrEqual(LARGE_OR_UI);
  });
});
