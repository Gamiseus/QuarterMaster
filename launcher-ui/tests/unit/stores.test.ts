import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const ok = <T>(data: T) => Promise.resolve({ status: "ok" as const, data });

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
  },
}));

import { commands } from "../../src/lib/bindings";
import { emitMock } from "../../src/lib/events";
import { connectStores } from "../../src/lib/stores";
import { game } from "../../src/lib/stores/game.svelte";
import { jobs } from "../../src/lib/stores/jobs.svelte";
import { mods } from "../../src/lib/stores/mods.svelte";

let disconnect: () => void = () => {};

beforeEach(() => {
  vi.stubEnv("VITE_MOCK", "1");
  disconnect = connectStores();
  vi.clearAllMocks();
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
    const mod = (id: string) => ({ id, name: id, version: "1", authors: [], category: null, enabled: true, patch_count: 0, installed_versions: [], is_workspace_linked: false, has_config: false });
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
