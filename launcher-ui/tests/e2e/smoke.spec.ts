import { expect, test } from "@playwright/test";

const PAGES = ["Play", "Mods", "Saves", "Diagnostics", "Settings"];

test("loads on the Play page", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1, name: "Play" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Minimize" })).toBeVisible();
});

test("sidebar navigates to each page", async ({ page }) => {
  await page.goto("/");
  const nav = page.getByRole("navigation", { name: "Main" });
  for (const name of PAGES) {
    await nav.getByRole("button", { name, exact: true }).click();
    await expect(page.getByRole("heading", { level: 1, name })).toBeVisible();
  }
  await nav.getByRole("switch", { name: "Mod Engine" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Mod Engine" })).toBeVisible();
  await expect(page.getByText("Mod Engine arrives in a later update.")).toBeVisible();
});

test("game status events update the launch button", async ({ page }) => {
  await page.goto("/");
  const button = page.getByTestId("launch-button");
  await expect(button).toHaveText("Launch");
  await expect(button).toBeEnabled();
  await page.evaluate(() => {
    window.__qmMockEmit?.({
      type: "game_status_changed",
      status: { stage: { stage: "running", pid: 1234 }, profile_id: "default", is_modded: true },
    });
  });
  await expect(button).toHaveText("Running");
  await expect(button).toBeDisabled();
});
