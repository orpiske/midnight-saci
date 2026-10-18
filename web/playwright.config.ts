import { defineConfig, devices } from "@playwright/test";

// The built client (`npm run build`) served by `vite preview`, with every
// API call answered by the in-test mock backend (e2e/mock.ts).
export default defineConfig({
  testDir: "e2e",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: "http://127.0.0.1:4173",
    trace: "retain-on-failure",
  },
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 900 } } },
    { name: "narrow", use: { ...devices["Pixel 7"] } },
  ],
  webServer: {
    command: "npx vite preview --host 127.0.0.1 --port 4173 --strictPort",
    url: "http://127.0.0.1:4173/ui/",
    reuseExistingServer: !process.env.CI,
  },
});
