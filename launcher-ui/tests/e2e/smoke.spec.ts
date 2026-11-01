import { expect, test } from "@playwright/test";

const PAGES = ["Mods", "Saves", "Diagnostics", "Settings"];

test("loads on the Mods screen with the title bar shell", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1, name: "Mods" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Minimize" })).toBeVisible();
  await expect(page.getByTestId("profile-name")).not.toHaveText("");
  await expect(page.getByTestId("save-slot-chip")).toBeVisible();
  await expect(page.getByTestId("health-summary")).toContainText(/patches ok|conflict/);
});

test("sidebar has no Play entry and its expanded state persists", async ({ page }) => {
  await page.goto("/");
  const nav = page.getByRole("navigation", { name: "Main" });
  await expect(nav.getByRole("button", { name: "Play" })).toHaveCount(0);
  await nav.getByRole("button", { name: "Expand sidebar" }).click();
  await expect(nav).toHaveClass(/expanded/);
  await page.reload();
  await expect(page.getByRole("navigation", { name: "Main" })).toHaveClass(/expanded/);
  await page.getByRole("button", { name: "Collapse sidebar" }).click();
  await expect(page.getByRole("navigation", { name: "Main" })).not.toHaveClass(/expanded/);
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
