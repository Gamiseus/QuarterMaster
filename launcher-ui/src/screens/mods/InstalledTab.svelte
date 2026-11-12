<script lang="ts">
  import { untrack } from "svelte";
  import { Button, EmptyState, ScrollRegion, Skeleton, Splitter } from "../../components/ui";
  import { cssVars } from "../../lib/actions/cssVars";
  import type { ModPatchStatus } from "../../lib/bindings";
  import { clampListWidth, defaultListWidth, LIST_MIN, loadSplit, saveSplit, widthClass } from "../../lib/layout/split";
  import { filterRows, type FilterChip } from "../../lib/modlist/filter";
  import { movedBySolver, resolvedPosition, rowKey } from "../../lib/modlist/order";
  import { assembleStatus } from "../../lib/modlist/status";
  import { collapseRows } from "../../lib/modlist/view";
  import { app } from "../../lib/stores/app.svelte";
  import { installs } from "../../lib/stores/installs.svelte";
  import { loadOrder } from "../../lib/stores/loadorder.svelte";
  import { mods } from "../../lib/stores/mods.svelte";
  import { modsView } from "./mods-view.svelte";
  import { patches } from "../../lib/stores/patches.svelte";
  import { profiles } from "../../lib/stores/profiles.svelte";
  import { selection } from "../../lib/stores/selection.svelte";
  import { updates } from "../../lib/stores/updates.svelte";
  import DetailPanel from "./detail/DetailPanel.svelte";
  import FilterBar from "./FilterBar.svelte";
  import { modList } from "./modlist-controller.svelte";
  import ModList from "./ModList.svelte";
  import SeparatorDialogs from "./SeparatorDialogs.svelte";
  import UpdatesNote from "./UpdatesNote.svelte";

  interface Props {
    onaddmod: () => void;
  }

  let { onaddmod }: Props = $props();

  let text = $state("");
  let chips = $state<Set<FilterChip>>(new Set());

  // Another screen asked for a filter (the "updates are ready" toast's Review).
  $effect(() => {
    const wanted = modsView.showChips;
    if (wanted === null) return;
    untrack(() => {
      chips = new Set(wanted);
      text = "";
      modsView.showChips = null;
    });
  });
  let separators = $state<SeparatorDialogs>();
  /** The list's keyboard cursor row. */
  let cursor = $state<string | null>(null);

  // Columns: the list keeps a readable width and the detail column gets the rest (spec 3.3).
  let windowWidth = $state(1280);
  let areaWidth = $state(0);
  const cls = $derived(widthClass(windowWidth));
  const narrow = $derived(cls === "narrow");
  /** The width the user dragged to for this window class; null means the default. */
  let chosen = $state<number | null>(null);
  $effect(() => {
    const c = cls;
    untrack(() => (chosen = loadSplit(c)));
  });
  const listWidth = $derived(clampListWidth(chosen ?? defaultListWidth(areaWidth), areaWidth));
  const listMax = $derived(clampListWidth(Number.POSITIVE_INFINITY, areaWidth));

  function setWidth(v: number) {
    chosen = v;
    saveSplit(cls, v);
  }

  function resetWidth() {
    chosen = null;
    saveSplit(cls, null);
  }

  // Narrow windows: details open over the list when a mod is picked.
  let overlayOpen = $state(false);
  $effect(() => {
    if (selection.focused && narrow) untrack(() => (overlayOpen = true));
  });

  function openFromRow(e: MouseEvent) {
    if (narrow && (e.target as HTMLElement).closest('[role="row"][data-key]')) overlayOpen = true;
  }

  function closeOverlay() {
    overlayOpen = false;
    const id = selection.focused;
    if (id) document.querySelector<HTMLElement>(`[role="row"][data-key="${CSS.escape(id)}"]`)?.focus();
  }

  // Rebuild from the stores whenever they change and no write is in flight.
  $effect(() => {
    const list = mods.list;
    const profile = profiles.active;
    if (list.status !== "ready" || !profile || modList.busy) return;
    const data = $state.snapshot(list.data);
    const doc = $state.snapshot(profile);
    untrack(() => modList.sync(data, doc));
  });

  const canMutate = $derived(app.startupDone);
  const ready = $derived(mods.list.status === "ready" && profiles.active !== null);
  const modItems = $derived(mods.list.status === "ready" ? mods.list.data : []);
  // Forget patch state of mods that are no longer installed.
  $effect(() => {
    if (mods.list.status !== "ready") return;
    const ids = mods.list.data.map((m) => m.id);
    untrack(() => patches.retain(ids));
  });
  const modsById = $derived(new Map(modItems.map((m) => [m.id, { ...m, enabled: modList.enabled[m.id] ?? m.enabled }])));
  const patchStates = $derived(
    new Map(
      modItems.flatMap((m): [string, ModPatchStatus[]][] => {
        const p = patches.get(m.id);
        return p.status === "ready" ? [[m.id, p.data]] : [];
      }),
    ),
  );
  // Row status needs every enabled mod's patch states, not only the one open in the detail panel.
  $effect(() => {
    if (loadOrder.simulation.status !== "ready") return;
    const ids = modItems.filter((m) => modsById.get(m.id)?.enabled && m.patch_count > 0).map((m) => m.id);
    untrack(() => patches.loadAll(ids));
  });
  const status = $derived(
    assembleStatus(
      modItems,
      loadOrder.simulation.status === "ready" ? loadOrder.simulation.data : null,
      updates.report.status === "ready" ? updates.report.data : null,
      patchStates,
    ),
  );
  const filtering = $derived(text.trim() !== "" || chips.size > 0);
  const visible = $derived(
    filtering ? filterRows(modList.rows, modsById, status, text, chips) : collapseRows(modList.rows, selection.selected),
  );
  const resolution = $derived(loadOrder.resolution.status === "ready" ? loadOrder.resolution.data : null);
  const positions = $derived(resolution ? resolvedPosition(resolution) : new Map());
  const moved = $derived(resolution ? movedBySolver(modList.rows, resolution) : new Set<string>());
  // Hidden rows must never be moved or toggled: drop them from the selection when the view changes.
  $effect(() => {
    if (!ready) return;
    const shown = new Set(visible.map(rowKey));
    const selected = selection.selected;
    if ([...selected].every((k) => shown.has(k))) return;
    untrack(() => (selection.selected = new Set([...selected].filter((k) => shown.has(k)))));
  });

  const focusedMod = $derived(selection.focused ? (modsById.get(selection.focused) ?? null) : null);

  function onWindowKey(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    if (e.key === "Escape" && narrow && overlayOpen && !e.defaultPrevented) {
      if (target?.closest?.('[role="menu"], [role="listbox"], .qm-dialog')) return;
      e.preventDefault();
      closeOverlay();
      return;
    }
    if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
    if (target?.closest?.('input, textarea, [contenteditable="true"], [role="dialog"]')) return;
    const key = e.key.toLowerCase();
    if (key === "z" && !e.shiftKey) {
      e.preventDefault();
      void modList.undo();
    } else if (key === "y" || (key === "z" && e.shiftKey)) {
      e.preventDefault();
      void modList.redo();
    }
  }
</script>

<svelte:window onkeydown={onWindowKey} bind:innerWidth={windowWidth} />

<div
  class={["installed", { narrow }]}
  bind:clientWidth={areaWidth}
  use:cssVars={{ "--qm-list-w": `${listWidth}px` }}
>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="list-col" onclick={openFromRow}>
    <FilterBar
      bind:text
      bind:chips
      {canMutate}
      compact={narrow}
      onaddseparator={() => separators?.edit(null)}
      {onaddmod}
      updateCount={updates.available.length}
      updateBlocked={installs.blockReason()}
      onupdateall={() => updates.installAll()}
    />
    <UpdatesNote />
    <ScrollRegion class="list-scroll">
      {#if !ready}
        <div class="skeleton" aria-busy="true" aria-label="Loading mods">
          {#each [1, 2, 3, 4, 5] as n (n)}
            <Skeleton />
          {/each}
        </div>
      {:else if modItems.length === 0}
        <EmptyState title="No mods installed" body="Mods you install show up here in load order.">
          {#snippet action()}
            <Button variant="primary" disabled={!canMutate} onclick={onaddmod}>Add your first mod</Button>
          {/snippet}
        </EmptyState>
      {:else}
        <ModList
          controller={modList}
          {visible}
          {modsById}
          {status}
          {positions}
          {moved}
          {canMutate}
          bind:cursor
          oneditseparator={(sep) => separators?.edit(sep)}
          onremoveseparator={(sep) => separators?.remove(sep)}
        />
        {#if filtering && !visible.some((r) => r.kind === "mod")}
          <p class="qm-muted none">No mods match the filter.</p>
        {/if}
      {/if}
    </ScrollRegion>
  </div>
  {#if !narrow}
    <Splitter
      bind:value={() => listWidth, setWidth}
      min={LIST_MIN}
      max={listMax}
      label="Resize the mod list"
      onreset={resetWidth}
    />
    <aside class="detail" aria-label="Mod details">
      <DetailPanel
        mod={focusedMod}
        {canMutate}
        onenabledchange={(enabled) => focusedMod && void modList.setEnabled([focusedMod.id], enabled)}
      />
    </aside>
  {:else if overlayOpen && focusedMod}
    <div class="overlay" role="dialog" aria-modal="false" aria-label="Mod details">
      <DetailPanel
        mod={focusedMod}
        {canMutate}
        onenabledchange={(enabled) => void modList.setEnabled([focusedMod.id], enabled)}
        onclose={closeOverlay}
      />
    </div>
  {/if}
</div>

<SeparatorDialogs bind:this={separators} {cursor} />

<style>
  .installed {
    position: relative;
    display: flex;
    flex: 1;
    min-height: 0;
    padding-top: var(--qm-space-4);
  }

  .list-col {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: var(--qm-space-3);
    width: var(--qm-list-w);
    min-width: 0;
    min-height: 0;
  }

  /* Alone in a narrow window, the list takes the whole width. */
  .narrow .list-col {
    flex: 1;
    width: auto;
  }

  .detail {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: var(--qm-detail-min);
    min-height: 0;
  }

  .overlay {
    position: absolute;
    top: var(--qm-space-4);
    right: 0;
    bottom: 0;
    z-index: var(--qm-z-panel);
    display: flex;
    flex-direction: column;
    width: min(var(--qm-detail-min) + var(--qm-space-6), 92%);
    border-left: 1px solid var(--qm-border-strong);
    background: var(--qm-canvas);
    padding-left: var(--qm-space-3);
  }

  .skeleton {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .none {
    margin: var(--qm-space-2) 0 0;
    font-size: var(--qm-text-13);
  }
</style>
