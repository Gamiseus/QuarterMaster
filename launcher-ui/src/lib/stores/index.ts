import { onEvent } from "../events";
import { app } from "./app.svelte";
import { catalog } from "./catalog.svelte";
import { diagnostics } from "./diagnostics.svelte";
import { fixes } from "./fixes.svelte";
import { game } from "./game.svelte";
import { jobs } from "./jobs.svelte";
import { loadOrder } from "./loadorder.svelte";
import { logs } from "./logs.svelte";
import { mods } from "./mods.svelte";
import { patches } from "./patches.svelte";
import { patchView } from "./patchview.svelte";
import { profiles } from "./profiles.svelte";
import { saves } from "./saves.svelte";
import { updates } from "./updates.svelte";

/** Routes backend events to every store. Returns an unsubscribe function. */
export function connectStores(): () => void {
  return onEvent((e) => {
    app.handle(e);
    profiles.handle(e);
    saves.handle(e);
    mods.handle(e);
    loadOrder.handle(e);
    updates.handle(e);
    catalog.handle(e);
    diagnostics.handle(e);
    patches.handle(e);
    patchView.handle(e);
    fixes.handle(e);
    game.handle(e);
    jobs.handle(e);
    logs.handle(e);
  });
}

/** Initial load of every store; failures are already routed to the UI. */
export function loadStores(): void {
  const loads = [
    app.load(),
    profiles.load(),
    saves.load(),
    mods.load(),
    loadOrder.load(),
    fixes.load(),
    updates.load(),
    game.load(),
    jobs.load(),
    app.awaitStartup(),
  ];
  for (const p of loads) {
    p.catch(() => {});
  }
}
