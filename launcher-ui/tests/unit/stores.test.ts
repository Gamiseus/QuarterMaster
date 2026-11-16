import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const ok = <T>(data: T) => Promise.resolve({ status: "ok" as const, data });
const resolution = (tag: string) => ({ ordered_mods: [], diagnostics: [], has_cycles: false, cycle_paths: [], rule_hierarchy: tag });
const simulation = (total: number) => ({ total_patches: total, applied_patches: total, broken_anchors: 0, conflicts: [], files: [] });
const report = (checked_at: string) => ({ checked_at, rate_limit_mode: "x", launcher: {}, mods: [] });
const patch = (patch_id: string) => ({ patch_id, name: null, target_file: "a.lua", state: "applied", conflicts: [] });
const summary = (id: string, is_active: boolean) => ({ id, name: id, description: "", enabled_count: 0, is_active, linked_save_profile_id: "default", updated_at: "", is_builtin: false });

vi.mock("../../src/lib/bindings", () => ({
  commands: {
    modsList: vi.fn(() => ok({ status: "ready", data: [] })),
    profilesList: vi.fn(() => ok([])),
    gameStatus: vi.fn(() =>
      ok({ stage: { stage: "idle" }, profile_id: null, is_modded: false }),
    ),
    jobsList: vi.fn(() => ok([])),
    appInfo: vi.fn(() => ok({ version: "1.0.0", data_dir: "", is_portable: false, game: { status: "loading" } })),
    settingsGet: vi.fn(() => ok(null)),
    startupStatus: vi.fn(() => ok({ done: true, recovered: null, attached_pid: null })),
    loadOrderGet: vi.fn(() => ok({ status: "ready", data: resolution("base") })),
    simulationGet: vi.fn(() => ok({ status: "ready", data: simulation(1) })),
    updatesGet: vi.fn(() => ok({ status: "ready", data: report("2026-01-01") })),
    modPatchesGet: vi.fn((id: string) => ok({ status: "ready", data: [patch(id)] })),
    profileGet: vi.fn((id: string) => ok({ id, name: id, enabled_mods: [] })),
    gameLaunch: vi.fn(() => ok({ stage: { stage: "deploying" }, profile_id: "p", is_modded: true })),
  },
}));

import { commands } from "../../src/lib/bindings";
import { emitMock } from "../../src/lib/events";
import { connectStores } from "../../src/lib/stores";
import { app } from "../../src/lib/stores/app.svelte";
import { game } from "../../src/lib/stores/game.svelte";
import { jobs } from "../../src/lib/stores/jobs.svelte";
import { loadOrder } from "../../src/lib/stores/loadorder.svelte";
import { mods } from "../../src/lib/stores/mods.svelte";
import { patches } from "../../src/lib/stores/patches.svelte";
import { prelaunch } from "../../src/lib/stores/prelaunch.svelte";
import { profiles } from "../../src/lib/stores/profiles.svelte";
import { selection } from "../../src/lib/stores/selection.svelte";
import { updates } from "../../src/lib/stores/updates.svelte";

/** Marks the active profile as the built-in Vanilla profile (or not). */
function setVanillaActive(on: boolean): void {
  profiles.list = profiles.list.map((p) => (p.is_active ? { ...p, is_builtin: on } : p));
}

let disconnect: () => void = () => {};

beforeEach(() => {
  vi.stubEnv("VITE_MOCK", "1");
  disconnect = connectStores();
  vi.clearAllMocks();
  patches.reset();
});

afterEach(() => {
  disconnect();
  vi.unstubAllEnvs();
});

describe("stores", () => {
  it("reload only the invalidated scope", async () => {
    emitMock({ type: "invalidated", scope: "mods" });
    await vi.waitFor(() => expect(commands.modsList).toHaveBeenCalledTimes(1));
    expect(commands.profilesList).toHaveBeenCalledTimes(0);
  });

  it("apply GameStatusChanged to game.status", () => {
    emitMock({
      type: "game_status_changed",
      status: { stage: { stage: "running", pid: 1234 }, profile_id: "default", is_modded: true },
    });
    expect(game.status.stage).toEqual({ stage: "running", pid: 1234 });
    expect(game.isBusy).toBe(true);
  });

  it("track job progress and finish", () => {
    emitMock({ type: "job_progress", id: 7, kind: "warm_mods", progress: 0.5, label: "Scanning mods" });
    expect(jobs.list.map((j) => j.id)).toEqual([7]);
    emitMock({ type: "job_finished", id: 7, kind: "warm_mods", error: null, output: null });
    expect(jobs.list).toEqual([]);
  });

  it("clear the game_running banner after a non-busy status load", async () => {
    game.banner = "The game is running.";
    vi.mocked(commands.gameStatus).mockImplementationOnce(() =>
      ok({ stage: { stage: "running", pid: 1 }, profile_id: "default", is_modded: true }),
    );
    await game.load();
    expect(game.banner).toBe("The game is running.");
    await game.load(); // default mock: idle
    expect(game.banner).toBeNull();
  });

  it("ignore a stale mods response", async () => {
    let resolveOld!: (v: unknown) => void;
    const mod = (id: string) => ({ id, name: id, version: "1", authors: [], category: null, enabled: true, patch_count: 0, installed_versions: [], is_workspace_linked: false, has_config: false, load_priority: 50 });
    vi.mocked(commands.modsList)
      .mockImplementationOnce(() => new Promise((r) => (resolveOld = r)) as never)
      .mockImplementationOnce(() => ok({ status: "ready", data: [mod("new")] }) as never);
    const first = mods.load();
    await mods.load();
    resolveOld({ status: "ok", data: { status: "ready", data: [mod("old")] } });
    await first;
    expect(mods.list.status === "ready" && mods.list.data.map((m) => m.id)).toEqual(["new"]);
  });

  it("merge job events received while jobs.load is in flight", async () => {
    let resolve!: (v: unknown) => void;
    vi.mocked(commands.jobsList).mockImplementationOnce(() => new Promise((r) => (resolve = r)) as never);
    const load = jobs.load();
    emitMock({ type: "job_progress", id: 21, kind: "warm_mods", progress: 0.2, label: "Scanning mods" });
    emitMock({ type: "job_finished", id: 22, kind: "warm_vanilla", error: null, output: null });
    resolve({
      status: "ok",
      data: [{ id: 22, kind: "warm_vanilla", progress: 0.9, label: "Old", cancelling: false }],
    });
    await load;
    expect(jobs.list.map((j) => [j.id, j.progress])).toEqual([[21, 0.2]]);
  });
});

const tick = () => new Promise((r) => setTimeout(r, 0));
const readyData = <T>(l: { status: string; data?: T }) => (l.status === "ready" ? l.data : undefined);

describe("load order store", () => {
  it("reloads on load_order only", async () => {
    emitMock({ type: "invalidated", scope: "saves" });
    emitMock({ type: "invalidated", scope: "profiles" });
    await tick();
    expect(commands.loadOrderGet).not.toHaveBeenCalled();
    expect(commands.simulationGet).not.toHaveBeenCalled();
    emitMock({ type: "invalidated", scope: "load_order" });
    await vi.waitFor(() => expect(commands.loadOrderGet).toHaveBeenCalledTimes(1));
    expect(commands.simulationGet).toHaveBeenCalledTimes(1);
    expect(commands.updatesGet).not.toHaveBeenCalled();
  });

  it("resolutionReady is false while the resolution loads, refreshes or a write may change it", async () => {
    loadOrder.resolution = { status: "loading" };
    expect(loadOrder.resolutionReady).toBe(false);
    await loadOrder.load();
    expect(loadOrder.resolutionReady).toBe(true);

    let resolveRefresh!: (v: unknown) => void;
    vi.mocked(commands.loadOrderGet).mockImplementationOnce(() => new Promise((r) => (resolveRefresh = r)) as never);
    const refresh = loadOrder.load();
    expect(loadOrder.resolution.status).toBe("ready");
    expect(loadOrder.resolutionReady).toBe(false);
    resolveRefresh({ status: "ok", data: { status: "ready", data: resolution("fresh") } });
    await refresh;
    expect(loadOrder.resolutionReady).toBe(true);

    let finishWrite!: () => void;
    let resolveReload!: (v: unknown) => void;
    vi.mocked(commands.loadOrderGet).mockImplementationOnce(() => new Promise((r) => (resolveReload = r)) as never);
    const write = loadOrder.whileChanging(() => new Promise<void>((r) => (finishWrite = r)));
    expect(loadOrder.resolutionReady).toBe(false);
    finishWrite();
    await write;
    // The write landed but the resolution reload it started has not.
    expect(loadOrder.resolutionReady).toBe(false);
    resolveReload({ status: "ok", data: { status: "ready", data: resolution("after-write") } });
    await vi.waitFor(() => expect(loadOrder.resolutionReady).toBe(true));
    expect(readyData<{ rule_hierarchy: string }>(loadOrder.resolution)?.rule_hierarchy).toBe("after-write");
  });

  it("canLaunch waits for a ready resolution unless launching vanilla", async () => {
    app.startupDone = true;
    profiles.list = [summary("p1", true)];
    await loadOrder.load();
    expect(prelaunch.canLaunch).toBe(true);

    let resolveRefresh!: (v: unknown) => void;
    vi.mocked(commands.loadOrderGet).mockImplementationOnce(() => new Promise((r) => (resolveRefresh = r)) as never);
    const refresh = loadOrder.load();
    expect(prelaunch.canLaunch).toBe(false);
    setVanillaActive(true);
    expect(prelaunch.canLaunch).toBe(true);
    setVanillaActive(false);
    resolveRefresh({ status: "ok", data: { status: "ready", data: resolution("x") } });
    await refresh;
    expect(prelaunch.canLaunch).toBe(true);
    profiles.list = [];
  });

  it("shows the last ready simulation of the same profile while a new one runs", async () => {
    profiles.list = [summary("p1", true), summary("p2", false)];
    loadOrder.forget();
    await loadOrder.load();
    expect(loadOrder.refreshing).toBe(false);
    // A profile write cleared the service's simulation.
    vi.mocked(commands.simulationGet).mockImplementationOnce(() => ok({ status: "loading" }) as never);
    await loadOrder.load();
    expect(loadOrder.simulation.status).toBe("loading");
    expect(readyData<{ total_patches: number }>(loadOrder.shown)?.total_patches).toBe(1);
    expect(loadOrder.refreshing).toBe(true);
    // The gates still see that it is not ready.
    expect(prelaunch.compileErrors).toEqual([]);
    expect(prelaunch.canLaunch).toBe(false);
    // Another profile: nothing of the old one is shown.
    profiles.list = [summary("p1", false), summary("p2", true)];
    expect(loadOrder.shown.status).toBe("loading");
    expect(loadOrder.refreshing).toBe(false);
    profiles.list = [];
  });

  it("drops a stale resolution and a stale simulation", async () => {
    let resolveOldRes!: (v: unknown) => void;
    let resolveOldSim!: (v: unknown) => void;
    vi.mocked(commands.loadOrderGet)
      .mockImplementationOnce(() => new Promise((r) => (resolveOldRes = r)) as never)
      .mockImplementationOnce(() => ok({ status: "ready", data: resolution("new") }) as never);
    vi.mocked(commands.simulationGet)
      .mockImplementationOnce(() => new Promise((r) => (resolveOldSim = r)) as never)
      .mockImplementationOnce(() => ok({ status: "ready", data: simulation(9) }) as never);
    const first = loadOrder.load();
    await loadOrder.load();
    resolveOldRes({ status: "ok", data: { status: "ready", data: resolution("old") } });
    resolveOldSim({ status: "ok", data: { status: "ready", data: simulation(1) } });
    await first;
    expect(readyData(loadOrder.resolution)?.rule_hierarchy).toBe("new");
    expect(readyData(loadOrder.simulation)?.total_patches).toBe(9);
  });
});

describe("game store", () => {
  it("asks for a launch without mods only when told to", async () => {
    await game.launch("p");
    expect(commands.gameLaunch).toHaveBeenLastCalledWith({ profile_id: "p", method: null });
    await game.launch("p", null, true);
    expect(commands.gameLaunch).toHaveBeenLastCalledWith({ profile_id: "p", method: null, without_mods: true });
  });
});

describe("updates store", () => {
  it("reloads on updates only", async () => {
    emitMock({ type: "invalidated", scope: "load_order" });
    await tick();
    expect(commands.updatesGet).not.toHaveBeenCalled();
    emitMock({ type: "invalidated", scope: "updates" });
    await vi.waitFor(() => expect(commands.updatesGet).toHaveBeenCalledTimes(1));
  });

  it("drops a stale report", async () => {
    let resolveOld!: (v: unknown) => void;
    vi.mocked(commands.updatesGet)
      .mockImplementationOnce(() => new Promise((r) => (resolveOld = r)) as never)
      .mockImplementationOnce(() => ok({ status: "ready", data: report("new") }) as never);
    const first = updates.load();
    await updates.load();
    resolveOld({ status: "ok", data: { status: "ready", data: report("old") } });
    await first;
    expect(readyData(updates.report)?.checked_at).toBe("new");
  });
});

describe("patches store", () => {
  it("is loading until a mod is loaded, then caches per mod", async () => {
    expect(patches.get("m1").status).toBe("loading");
    await patches.load("m1");
    expect(readyData(patches.get("m1"))?.[0]?.patch_id).toBe("m1");
    expect(patches.get("m2").status).toBe("loading");
  });

  it("refetches every loaded mod on load_order and mods, keeping old data until it lands", async () => {
    await patches.load("m1");
    await patches.load("m2");
    vi.mocked(commands.modPatchesGet).mockClear();
    vi.mocked(commands.modPatchesGet).mockImplementation((id: string) =>
      ok({ status: "ready", data: [patch(`${id}-new`)] }) as never,
    );
    emitMock({ type: "invalidated", scope: "load_order" });
    expect(readyData(patches.get("m1"))?.[0]?.patch_id).toBe("m1");
    await vi.waitFor(() => expect(readyData(patches.get("m1"))?.[0]?.patch_id).toBe("m1-new"));
    await vi.waitFor(() => expect(readyData(patches.get("m2"))?.[0]?.patch_id).toBe("m2-new"));
    expect(vi.mocked(commands.modPatchesGet).mock.calls.map((c) => c[0]).sort()).toEqual(["m1", "m2"]);

    vi.mocked(commands.modPatchesGet).mockClear();
    emitMock({ type: "invalidated", scope: "mods" });
    await vi.waitFor(() => expect(commands.modPatchesGet).toHaveBeenCalledTimes(2));
    vi.mocked(commands.modPatchesGet).mockImplementation((id: string) => ok({ status: "ready", data: [patch(id)] }) as never);
  });

  it("loadAll fetches only mods without an entry or a request in flight", async () => {
    await patches.load("m1");
    vi.mocked(commands.modPatchesGet).mockClear();
    patches.loadAll(["m1", "m2", "m2"]);
    expect(vi.mocked(commands.modPatchesGet).mock.calls.map((c) => c[0])).toEqual(["m2"]);
    await vi.waitFor(() => expect(patches.get("m2").status).toBe("ready"));
    patches.loadAll(["m1", "m2"]);
    expect(commands.modPatchesGet).toHaveBeenCalledTimes(1);

    // Mods loaded through loadAll are refetched on invalidation too.
    emitMock({ type: "invalidated", scope: "mods" });
    await vi.waitFor(() => expect(commands.modPatchesGet).toHaveBeenCalledTimes(3));
  });

  it("ignores other scopes", async () => {
    await patches.load("m3");
    emitMock({ type: "invalidated", scope: "updates" });
    emitMock({ type: "invalidated", scope: "profiles" });
    expect(patches.get("m3").status).toBe("ready");
  });

  it("drops a response that was in flight when the cache was cleared", async () => {
    let resolveOld!: (v: unknown) => void;
    vi.mocked(commands.modPatchesGet)
      .mockImplementationOnce(() => new Promise((r) => (resolveOld = r)) as never)
      .mockImplementationOnce(() => ok({ status: "ready", data: [patch("fresh")] }) as never);
    const first = patches.load("m4");
    emitMock({ type: "invalidated", scope: "mods" });
    resolveOld({ status: "ok", data: { status: "ready", data: [patch("stale")] } });
    await first;
    await vi.waitFor(() => expect(readyData(patches.get("m4"))?.[0]?.patch_id).toBe("fresh"));
  });

  it("lands a response only under its own mod", async () => {
    let resolveA!: (v: unknown) => void;
    vi.mocked(commands.modPatchesGet).mockImplementationOnce(() => new Promise((r) => (resolveA = r)) as never);
    const a = patches.load("mA");
    await patches.load("mB");
    expect(readyData(patches.get("mB"))?.[0]?.patch_id).toBe("mB");
    expect(patches.get("mA").status).toBe("loading");
    resolveA({ status: "ok", data: { status: "ready", data: [patch("mA")] } });
    await a;
    expect(readyData(patches.get("mA"))?.[0]?.patch_id).toBe("mA");
    expect(readyData(patches.get("mB"))?.[0]?.patch_id).toBe("mB");
  });

  it("drops a stale response for the same mod", async () => {
    let resolveOld!: (v: unknown) => void;
    vi.mocked(commands.modPatchesGet)
      .mockImplementationOnce(() => new Promise((r) => (resolveOld = r)) as never)
      .mockImplementationOnce(() => ok({ status: "ready", data: [patch("new")] }) as never);
    const first = patches.load("m5");
    await patches.load("m5");
    resolveOld({ status: "ok", data: { status: "ready", data: [patch("old")] } });
    await first;
    expect(readyData(patches.get("m5"))?.[0]?.patch_id).toBe("new");
  });

  it("records a failed load as an error and a retry clears it", async () => {
    vi.mocked(commands.modPatchesGet).mockImplementationOnce(
      () => Promise.resolve({ status: "error", error: { kind: "io", message: "Disk failed.", details: null } }) as never,
    );
    await expect(patches.load("m6")).rejects.toThrow("Disk failed.");
    expect(patches.get("m6")).toEqual({ status: "error", message: "Disk failed." });
    const retry = patches.load("m6");
    expect(patches.get("m6").status).toBe("loading");
    await retry;
    expect(readyData(patches.get("m6"))?.[0]?.patch_id).toBe("m6");
  });
});

describe("profiles store", () => {
  it("loads the active profile document", async () => {
    vi.mocked(commands.profilesList).mockImplementationOnce(
      () => ok([summary("a", false), summary("b", true)]) as never,
    );
    await profiles.load();
    expect(commands.profileGet).toHaveBeenCalledWith("b");
    expect(profiles.active?.id).toBe("b");
    expect(profiles.activeSummary?.id).toBe("b");
  });

  it("reloads the list and active document on a profiles event", async () => {
    vi.mocked(commands.profilesList).mockImplementationOnce(() => ok([summary("p1", true)]) as never);
    emitMock({ type: "invalidated", scope: "profiles" });
    await vi.waitFor(() => expect(profiles.active?.id).toBe("p1"));
    expect(commands.profilesList).toHaveBeenCalledTimes(1);
  });

  it("clears the active document while a different profile loads, and keeps it null on failure", async () => {
    vi.mocked(commands.profilesList).mockImplementationOnce(() => ok([summary("x", true)]) as never);
    await profiles.load();
    expect(profiles.active?.id).toBe("x");
    vi.mocked(commands.profilesList).mockImplementationOnce(() => ok([summary("y", true)]) as never);
    vi.mocked(commands.profileGet).mockImplementationOnce(
      () => Promise.resolve({ status: "error", error: { kind: "not_found", message: "gone", details: null } }) as never,
    );
    await profiles.load().catch(() => {});
    expect(profiles.activeSummary?.id).toBe("y");
    expect(profiles.active).toBeNull();
  });

  it("drops a stale active document response", async () => {
    vi.mocked(commands.profilesList).mockImplementationOnce(() => ok([summary("s", true)]) as never);
    await profiles.load();
    let resolveOld!: (v: unknown) => void;
    vi.mocked(commands.profileGet)
      .mockImplementationOnce(() => new Promise((r) => (resolveOld = r)) as never)
      .mockImplementationOnce(() => ok({ id: "s", name: "fresh", enabled_mods: [] }) as never);
    const first = profiles.loadActive();
    await profiles.loadActive();
    resolveOld({ status: "ok", data: { id: "s", name: "stale", enabled_mods: [] } });
    await first;
    expect(profiles.active?.name).toBe("fresh");
  });

  it("reloads only the active document on load_order", async () => {
    vi.mocked(commands.profilesList).mockImplementationOnce(() => ok([summary("a", true)]) as never);
    await profiles.load();
    vi.mocked(commands.profilesList).mockClear();
    vi.mocked(commands.profileGet).mockClear();
    emitMock({ type: "invalidated", scope: "load_order" });
    await vi.waitFor(() => expect(commands.profileGet).toHaveBeenCalledTimes(1));
    expect(commands.profilesList).not.toHaveBeenCalled();
  });
});

describe("selection store", () => {
  const order = ["a", "b", "c", "d", "e"];

  it("replaces, toggles and clears", () => {
    selection.clear();
    selection.select("b", "replace", order);
    selection.select("d", "toggle", order);
    expect([...selection.selected]).toEqual(["b", "d"]);
    expect(selection.focused).toBe("d");
    selection.select("b", "toggle", order);
    expect([...selection.selected]).toEqual(["d"]);
    selection.select("e", "replace", order);
    expect([...selection.selected]).toEqual(["e"]);
    selection.clear();
    expect(selection.selected.size).toBe(0);
    expect(selection.focused).toBeNull();
  });

  it("range selects every id between focused and target in order", () => {
    selection.clear();
    selection.select("b", "replace", order);
    selection.select("d", "range", order);
    expect([...selection.selected]).toEqual(["b", "c", "d"]);
    selection.select("a", "range", order);
    expect([...selection.selected]).toEqual(["a", "b"]);
  });

  it("range with an anchor missing from the order falls back to replace", () => {
    selection.clear();
    selection.select("zzz", "replace", order);
    selection.select("c", "range", order);
    expect([...selection.selected]).toEqual(["c"]);
    expect(selection.focused).toBe("c");
  });

  it("range without a focused row selects just the target", () => {
    selection.clear();
    selection.select("c", "range", order);
    expect([...selection.selected]).toEqual(["c"]);
  });
});

describe("app store", () => {
  it("sets startupDone once startup_status reports done", async () => {
    app.startupDone = false;
    vi.mocked(commands.startupStatus)
      .mockImplementationOnce(() => ok({ done: false, recovered: null, attached_pid: null }) as never)
      .mockImplementationOnce(() => ok({ done: true, recovered: null, attached_pid: null }) as never);
    const wait = app.awaitStartup();
    await tick();
    expect(app.startupDone).toBe(false);
    await wait;
    expect(app.startupDone).toBe(true);
  });

  it("defaults to the mods page", () => {
    expect(app.page).toBe("mods");
  });
});
