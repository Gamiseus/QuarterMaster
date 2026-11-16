import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "tests/e2e",
  fullyParallel: false,
  reporter: "list",
  // Visual baselines are valid for the Windows 11 reference environment (Segoe UI Variable body text;
  // only the heading font is bundled). On a different setup, regenerate with --update-snapshots.
  expect: { toHaveScreenshot: { maxDiffPixelRatio: 0.002 } },
  use: {
    // 127.0.0.1, not localhost: on Windows localhost can resolve to ::1 or 127.0.0.1, and a request
    // sent to the address the server is not bound to is refused (a blank page in a random test).
    baseURL: "http://127.0.0.1:5173",
    trace: "retain-on-failure",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } } },
  ],
  // The built mock bundle served by vite preview on 5173 (vite.e2e.config.ts), not the dev server:
  // a page load fetches a few files instead of one per source module, and the server closes each
  // socket, so back-to-back runs do not fail page loads with net::ERR_ADDRESS_IN_USE.
  webServer: {
    command: "npm run e2e:serve",
    url: "http://127.0.0.1:5173",
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
