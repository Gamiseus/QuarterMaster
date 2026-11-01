import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "tests/e2e",
  fullyParallel: false,
  reporter: "list",
  // Visual baselines are valid for the Windows 11 reference environment (Segoe UI Variable body text;
  // only the heading font is bundled). On a different setup, regenerate with --update-snapshots.
  expect: { toHaveScreenshot: { maxDiffPixelRatio: 0.002 } },
  use: {
    baseURL: "http://localhost:5173",
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } } }],
  webServer: {
    command: "npm run dev:mock",
    port: 5173,
    reuseExistingServer: false,
    timeout: 60_000,
  },
});
