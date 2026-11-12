<script lang="ts">
  import { untrack } from "svelte";
  import { Button, EmptyState, Panel, ScrollRegion, Skeleton, Tabs } from "../../components/ui";
  import type { SaveSummary } from "../../lib/bindings";
  import { app } from "../../lib/stores/app.svelte";
  import { game } from "../../lib/stores/game.svelte";
  import { saves } from "../../lib/stores/saves.svelte";
  import BackupsTab from "./BackupsTab.svelte";
  import DossierTab from "./DossierTab.svelte";
  import RibbonsTab from "./RibbonsTab.svelte";
  import SlotHeader from "./SlotHeader.svelte";
  import SoldiersTab from "./SoldiersTab.svelte";

  const TABS = [
    { value: "dossier", label: "Dossier" },
    { value: "ribbons", label: "Ribbons" },
    { value: "soldiers", label: "Soldiers" },
    { value: "backups", label: "Backups" },
  ];

  let tab = $state("dossier");
  /** The last dossier shown per slot, kept on screen while a refetch runs after an invalidation. */
  let shown = $state<{ id: string; data: SaveSummary } | null>(null);

  const slot = $derived(saves.list?.find((s) => s.profile.id === saves.current) ?? null);
  const id = $derived(slot?.profile.id ?? null);
  const detail = $derived(id ? saves.detail.get(id) : undefined);

  $effect(() => {
    const target = id;
    if (target && detail === undefined) untrack(() => void saves.loadDetail(target));
  });

  $effect(() => {
    if (id && detail?.status === "ready") shown = { id, data: detail.data };
  });

  const summary = $derived(detail?.status === "ready" ? detail.data : shown && shown.id === id ? shown.data : null);

  const startup = $derived(app.startupDone ? null : "Available once startup has finished.");
  const activeBusy = $derived(slot?.is_active && game.isBusy);
  const backupBlocked = $derived(startup ?? (activeBusy ? "Close the game to back up the active slot." : null));
  const restoreBlocked = $derived(startup ?? (game.isBusy ? "Close the game to restore a backup." : null));
  const tabLabel = $derived(TABS.find((t) => t.value === tab)?.label ?? "");
</script>

<Panel corners class="slot-detail">
  {#if !slot}
    {#if saves.list === null}
      <div aria-busy="true" aria-label="Loading the slot"><Skeleton lines={6} /></div>
    {:else}
      <EmptyState title="No save slots" body="Save slots appear here once the game folder is set up." />
    {/if}
  {:else}
    <SlotHeader {slot} {summary} />
    <Tabs fill bind:value={tab} tabs={TABS}>
      {#snippet content(value)}
        <ScrollRegion class="body" label={tabLabel}>
          {#if value === "backups"}
            <BackupsTab profile={slot.profile} {backupBlocked} {restoreBlocked} deleteBlocked={startup} />
          {:else if summary}
            {#if value === "ribbons"}
              <RibbonsTab {summary} />
            {:else if value === "soldiers"}
              <SoldiersTab {summary} />
            {:else}
              <DossierTab {summary} profile={slot.profile} />
            {/if}
          {:else if detail?.status === "error"}
            <div role="alert" class="error">
              <p>{detail.message}</p>
              <Button size="sm" onclick={() => id && saves.loadDetail(id)}>Retry</Button>
            </div>
          {:else}
            <div aria-busy="true" aria-label="Loading the dossier"><Skeleton lines={6} /></div>
          {/if}
        </ScrollRegion>
      {/snippet}
    </Tabs>
  {/if}
</Panel>

<style>
  :global(.slot-detail) {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--qm-space-3);
    min-height: 0;
    min-width: 0;
  }

  :global(.slot-detail .body) {
    flex: 1;
    min-height: 0;
    padding-top: var(--qm-space-3);
  }

  :global(.slot-detail .body > *) {
    max-width: calc(var(--qm-measure) * 1.4);
  }

  .error p {
    margin: 0 0 var(--qm-space-2);
  }
</style>
