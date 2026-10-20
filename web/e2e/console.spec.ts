import AxeBuilder from "@axe-core/playwright";
import { type Page, type TestInfo, expect, test } from "@playwright/test";

import * as fx from "./fixtures";
import { mockBackend } from "./mock";

async function accessible(page: Page, testInfo: TestInfo, name: string) {
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  await page.screenshot({ path: testInfo.outputPath(`${name}.png`), fullPage: true });
  expect(results.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(", ")}`)).toEqual([]);
}

test("the runs list shows live and finished runs", async ({ page }, testInfo) => {
  await mockBackend(page);
  await page.goto("/ui/");
  await expect(page).toHaveURL(/\/ui\/runs$/);
  await expect(page.getByRole("heading", { name: "Runs" })).toBeVisible();
  const rows = page.getByRole("row");
  await expect(rows.filter({ hasText: "Checkout flow" })).toContainText("Running");
  await expect(rows.filter({ hasText: "Service health" })).toContainText("Failed");
  await accessible(page, testInfo, "runs");
});

test("a failed run shows its criteria and the evidence they cite", async ({ page }, testInfo) => {
  await mockBackend(page);
  await page.goto(`/ui/runs/${fx.RUN}`);
  await expect(page.getByRole("heading", { name: "Service health" })).toBeVisible();
  await expect(page.getByText("Cleanup did not run")).toBeVisible();
  await expect(page.getByText("quay.io/saci/saci-worker:0.3.0")).toBeVisible();
  await expect(page.getByTitle(fx.report.worker_image?.digest ?? "")).toHaveText("sha256:9f86d081884c…");
  const detail = page.getByRole("region", { name: "Test 1.2: List orders" });
  await expect(detail.getByText("Contradicted")).toBeVisible();
  await expect(detail.getByText("Mechanical check")).toBeVisible();
  await detail.getByRole("button", { name: /ev-2/ }).first().click();
  await expect(page).toHaveURL(/evidence=ev-2/);
  const shown = page.getByRole("region", { name: /ev-2 · HTTP request/ });
  await expect(shown.getByText("500", { exact: true })).toBeVisible();
  await expect(shown.getByText("␛")).toBeVisible();
  await shown.getByRole("tab", { name: "Headers" }).click();
  await expect(shown.getByText("[REDACTED:TARGET_TOKEN]")).toBeVisible();
  await accessible(page, testInfo, "run-results");

  await page.reload();
  await expect(page.getByRole("region", { name: /ev-2 · HTTP request/ })).toBeVisible();
});

test("the activity view follows a live run until it finishes", async ({ page }, testInfo) => {
  await mockBackend(page);
  await page.goto(`/ui/runs/${fx.LIVE}?tab=activity`);
  const feed = page.getByRole("region", { name: "Activity" });
  await expect(feed.getByText("Run passed: every test passed")).toBeVisible();
  await expect(feed.getByText("Finished, no more events")).toBeVisible();
  await expect(feed.getByRole("listitem")).toHaveCount(fx.liveEvents.length);
  await expect(page.locator(".run-header .badge").first()).toHaveText("Passed");
  await accessible(page, testInfo, "run-activity");

  await feed.getByRole("button", { name: "View evidence" }).click();
  await expect(page).toHaveURL(/evidence=ev-1/);
});

async function addPlan(page: Page) {
  await page.goto("/ui/runs/new");
  await page.locator('input[type="file"][accept]').setInputFiles({
    name: "plan.md",
    mimeType: "text/markdown",
    buffer: Buffer.from(fx.PLAN_SOURCE),
  });
  await expect(page.getByLabel("Path in the bundle")).toHaveValue("plan.md");
  await expect(page.getByLabel("Environment")).toHaveValue("local");
}

test("a submission without an answer is retried with the same key", async ({ page }, testInfo) => {
  const mock = await mockBackend(page, { submit: [(route) => route.abort("connectionreset")] });
  await addPlan(page);
  await accessible(page, testInfo, "new-run");
  await page.getByRole("button", { name: "Start run" }).click();
  await expect(page.getByText("The backend did not confirm the submission")).toBeVisible();
  await page.getByRole("button", { name: "Retry start" }).click();
  await expect(page).toHaveURL(new RegExp(`/ui/runs/${fx.LIVE}$`));
  const keys = mock.requests
    .filter((r) => r.method() === "POST" && r.url().endsWith("/api/v1/runs"))
    .map((r) => r.headers()["idempotency-key"]);
  expect(keys).toHaveLength(2);
  expect(keys[0]).toBeTruthy();
  expect(keys[1]).toBe(keys[0]);
});

test("an invalid plan shows each diagnostic at its line", async ({ page }) => {
  await mockBackend(page, {
    submit: [
      (route) =>
        route.fulfill({
          status: 422,
          contentType: "application/json",
          body: JSON.stringify({
            error: {
              code: "validation_failed",
              message: "the plan is invalid",
              details: ["plan.md:5: Test 1.1 has no expected result"],
              diagnostics: [{ document: "plan.md", line: 5, message: "Test 1.1 has no expected result" }],
            },
          }),
        }),
    ],
  });
  await addPlan(page);
  await page.getByRole("button", { name: "Start run" }).click();
  const callout = page.getByRole("alert").filter({ hasText: "The backend did not accept the plan" });
  await expect(callout).toContainText("Test 1.1 has no expected result");
  await expect(page.locator(".code-line.diagnostic")).toHaveCount(1);
});

test("approving an action sends the digest that was reviewed", async ({ page }, testInfo) => {
  const mock = await mockBackend(page);
  await page.goto("/ui/actions");
  await page.getByRole("button", { name: "escalate" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText(`Escalate ${fx.RUN}`)).toBeVisible();
  await accessible(page, testInfo, "action-dialog");
  await dialog.getByLabel("Note (kept with the decision)").fill("looks right");
  await dialog.getByRole("button", { name: "Approve and send" }).click();
  await expect(dialog.getByText("Queued")).toBeVisible();
  const approval = mock.requests.find((r) => r.url().endsWith("/approval"));
  expect(approval?.postDataJSON()).toEqual({ digest: fx.pendingAction().digest, note: "looks right" });
});

test("an action that changed under review is shown again, not decided", async ({ page }) => {
  const mock = await mockBackend(page);
  await page.goto(`/ui/actions?run=${fx.RUN}&action=escalate`);
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("button", { name: "Reject" })).toBeVisible();
  const [action] = mock.actions;
  if (action) action.digest = "0".repeat(64);
  await dialog.getByRole("button", { name: "Reject" }).click();
  await expect(dialog.getByText("The action changed")).toBeVisible();
  await expect(dialog.getByText(`${"0".repeat(16)}…`)).toBeVisible();
});

test("a required token is asked for and kept in memory only", async ({ page }, testInfo) => {
  await mockBackend(page, { token: "s3cret-token" });
  await page.goto("/ui/runs");
  await expect(page.getByRole("heading", { name: "Connect to the backend" })).toBeVisible();
  await accessible(page, testInfo, "token");
  await page.getByLabel("Client token").fill("s3cret-token");
  await page.getByRole("button", { name: "Connect" }).click();
  await expect(page.getByRole("heading", { name: "Runs" })).toBeVisible();
  await expect(page.getByRole("row").filter({ hasText: "Service health" })).toBeVisible();
  const stored = await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }) + document.cookie);
  expect(stored).not.toContain("s3cret-token");
  expect(page.url()).not.toContain("s3cret-token");
});

test("the connection screen passes accessibility checks", async ({ page }, testInfo) => {
  await mockBackend(page);
  await page.goto("/ui/connection");
  await expect(page.getByRole("main").getByText("Reachable", { exact: true })).toBeVisible();
  await accessible(page, testInfo, "connection");
});

test("nothing is blocked by the backend's Content-Security-Policy", async ({ page }) => {
  const mock = await mockBackend(page);
  await page.goto(`/ui/runs/${fx.RUN}?evidence=ev-2&unit=1.2`);
  await expect(page.getByRole("region", { name: /ev-2 · HTTP request/ })).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  expect(mock.blocked).toEqual([]);
});
