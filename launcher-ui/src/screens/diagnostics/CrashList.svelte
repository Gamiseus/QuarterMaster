<script lang="ts">
  import { Badge, Button, EmptyState, Inset, Panel, ScrollRegion, Skeleton, StencilHeading } from "../../components/ui";
  import type { CrashReport } from "../../lib/bindings";
  import { formatTimestamp } from "../../lib/saves/format";
  import { app } from "../../lib/stores/app.svelte";
  import { diagnostics } from "../../lib/stores/diagnostics.svelte";
  import { game } from "../../lib/stores/game.svelte";
  import { mods } from "../../lib/stores/mods.svelte";
  import { profiles } from "../../lib/stores/profiles.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";

  let pending = $state<string | null>(null);

  const modName = (id: string) => (mods.list.status === "ready" ? mods.list.data.find((m) => m.id === id)?.name : undefined) ?? id;
  const enabled = $derived(new Set(profiles.active?.enabled_mods ?? []));
  const profileName = $derived(profiles.active?.name ?? "the active profile");
  const canChange = $derived(app.startupDone && !game.isBusy);

  /** An adjusted copy is the profile's fix, not its base mod's: the offer pauses the fix (D58). */
  const copyName = (c: CrashReport) => c.suspected_patch_name ?? c.suspected_patch_id ?? "the adjusted copy";

  async function disable(c: CrashReport) {
    pending = c.id;
    const label = `${modName(c.suspected_mod_id ?? "")} in ${profileName}`;
    try {
      await diagnostics.disableSuspect(c.id);
      toasts.info(c.suspected_adjusted_copy ? `Paused ${copyName(c)}. Launch the game to test without it.` : `Disabled ${label}.`);
    } catch {
      // call() already showed the banner or toast.
    } finally {
      pending = null;
    }
  }

  const where = (c: CrashReport) =>
    c.crashing_file ? `${c.crashing_file}${c.crashing_line !== null ? `:${c.crashing_line}` : ""}` : null;
</script>

<Panel class="crashes">
  <StencilHeading level={2}>Crashes</StencilHeading>
  {#if diagnostics.crashes === null}
    <div aria-busy="true" aria-label="Loading crashes"><Skeleton lines={4} /></div>
  {:else if diagnostics.crashes.length === 0}
    <EmptyState title="No crashes recorded." body="Crash reports from the game show up here with the mod that likely caused them." />
  {:else}
    <ScrollRegion class="card-scroll" label="Crashes">
      <ul class="list" aria-label="Crash reports">
        {#each diagnostics.crashes as c (c.id)}
          {@const suspect = c.suspected_mod_id}
          <li>
            <Inset class="crash">
              <div class="top">
                <span class="qm-muted when">{formatTimestamp(c.timestamp)}</span>
                <Badge>{c.crash_kind.replaceAll("_", " ")}</Badge>
              </div>
              <p class="message">{c.error_message}</p>
              {#if where(c)}<p class="qm-muted where">{where(c)}</p>{/if}
              {#if suspect}
                <div class="suspect">
                  <span>
                    Suspected: {modName(suspect)}{c.suspected_patch_name ? `, patch ${c.suspected_patch_name}` : ""}
                  </span>
                  {#if c.suspected_adjusted_copy}
                    <Button size="sm" disabled={!canChange || pending !== null} onclick={() => disable(c)}>
                      Pause {copyName(c)}
                    </Button>
                  {:else if enabled.has(suspect)}
                    <Button size="sm" disabled={!canChange || pending !== null} onclick={() => disable(c)}>
                      Disable {modName(suspect)} in {profileName}
                    </Button>
                  {:else}
                    <Badge tone="neutral">Disabled</Badge>
                  {/if}
                </div>
              {/if}
            </Inset>
          </li>
        {/each}
      </ul>
    </ScrollRegion>
  {/if}
</Panel>

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* Each report is its own inset box, so neighbouring reports never run together. */
  .list :global(.crash) {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  .top,
  .suspect {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .suspect {
    justify-content: space-between;
    font-size: var(--qm-text-13);
  }

  .when,
  .where {
    font-size: var(--qm-text-12);
  }

  .message {
    margin: 0;
    font-size: var(--qm-text-13);
    overflow-wrap: anywhere;
  }

  .where {
    margin: 0;
    font-family: var(--qm-font-mono);
  }
</style>
