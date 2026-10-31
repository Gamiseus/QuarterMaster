import { onEvent } from "../events";
import { app } from "./app.svelte";
import { game } from "./game.svelte";
import { jobs } from "./jobs.svelte";
import { logs } from "./logs.svelte";
import { mods } from "./mods.svelte";
import { profiles } from "./profiles.svelte";

/** Routes backend events to every store. Returns an unsubscribe function. */
export function connectStores(): () => void {
  return onEvent((e) => {
    app.handle(e);
    profiles.handle(e);
    mods.handle(e);
    game.handle(e);
    jobs.handle(e);
    logs.handle(e);
  });
}

/** Initial load of every store; failures are already routed to the UI. */
export function loadStores(): void {
  for (const p of [app.load(), profiles.load(), mods.load(), game.load(), jobs.load(), app.awaitStartup()]) {
    p.catch(() => {});
  }
}
