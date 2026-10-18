/// <reference types="vitest/config" />
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";

import react from "@vitejs/plugin-react";
import { type Plugin, defineConfig } from "vite";

const require = createRequire(import.meta.url);

/** Runtime packages whose license texts ship with the built client. */
const LICENSED = [
  "@fontsource-variable/bricolage-grotesque",
  "@fontsource/jetbrains-mono",
  "lucide-react",
  "react",
  "react-dom",
  "scheduler",
];

function licenseText(name: string): string {
  const manifest = require.resolve(`${name}/package.json`);
  const { version, license } = JSON.parse(readFileSync(manifest, "utf8")) as {
    version: string;
    license: string;
  };
  const text = readFileSync(join(dirname(manifest), "LICENSE"), "utf8").trim();
  return `${name} ${version} (${license})\n\n${text}\n`;
}

/** Writes `licenses.txt` next to the client: the self-hosted fonts' SIL
 * Open Font License notices and the other bundled packages' licenses. */
function licenses(): Plugin {
  return {
    name: "saci-licenses",
    apply: "build",
    generateBundle() {
      const separator = `\n${"=".repeat(72)}\n\n`;
      this.emitFile({
        type: "asset",
        fileName: "licenses.txt",
        source: LICENSED.map(licenseText).join(separator),
      });
    },
  };
}

const backend = process.env.SACI_BACKEND ?? "http://127.0.0.1:8080";

export default defineConfig({
  base: "/ui/",
  plugins: [react(), licenses()],
  build: {
    outDir: "dist",
    assetsDir: "assets",
    sourcemap: false,
    chunkSizeWarningLimit: 800,
  },
  server: {
    // Development only: the API stays same-origin through this proxy, so
    // the backend needs no CORS configuration.
    proxy: { "/api": { target: backend, changeOrigin: false } },
    // The starter plan is imported from docs/templates.
    fs: { allow: [".."] },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
