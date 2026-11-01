<script lang="ts">
  import { untrack } from "svelte";
  import { Button, Dialog, EmptyState, Skeleton, StencilHeading } from "../../components/ui";
  import type { ModPatchStatus, ProfileSeparator } from "../../lib/bindings";
  import { filterRows, type FilterChip } from "../../lib/modlist/filter";
  import { movedBySolver, resolvedPosition, rowKey } from "../../lib/modlist/order";
  import { assembleStatus } from "../../lib/modlist/status";
  import { collapseRows } from "../../lib/modlist/view";
  import { app } from "../../lib/stores/app.svelte";
  import { loadOrder } from "../../lib/stores/loadorder.svelte";
  import { mods } from "../../lib/stores/mods.svelte";
  import { patches } from "../../lib/stores/patches.svelte";
  import { profiles } from "../../lib/stores/profiles.svelte";
  import { selection } from "../../lib/stores/selection.svelte";
  import { updates } from "../../lib/stores/updates.svelte";
  import FilterBar from "./FilterBar.svelte";
  import DetailPanel from "./detail/DetailPanel.svelte";
  import { modList } from "./modlist-controller.svelte";
  import ModList from "./ModList.svelte";
  import SeparatorDialog from "./SeparatorDialog.svelte";

  let text = $state("");
  let chips = $state<Set<FilterChip>>(new Set());
  let editOpen = $state(false);
  let editing = $state<ProfileSeparator | null>(null);
  let removeOpen = $state(false);
  let removing = $state<ProfileSeparator | null>(null);
  /** The list's keyboard cursor row. */
  let cursor = $state<string | null>(null);

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
    filtering
      ? filterRows(modList.rows, modsById, status, text, chips)
      : collapseRows(modList.rows, selection.selected),
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

  function addSeparator() {
    editing = null;
    editOpen = true;
  }

  function editSeparator(sep: ProfileSeparator) {
    editing = sep;
    editOpen = true;
  }

  function submitSeparator(title: string, color: string) {
    if (editing) void modList.updateSeparator(editing.id, { title, color });
    else {
      // Before the cursor row, else the focused row; at the end only when that row is gone.
      const key = cursor ?? selection.focused;
      const exists = key !== null && modList.rows.some((r) => rowKey(r) === key);
      void modList.addSeparator(title, color, exists ? key : null);
    }
  }

  function askRemove(sep: ProfileSeparator) {
    removing = sep;
    removeOpen = true;
  }

  function confirmRemove() {
    if (removing) void modList.removeSeparator(removing.id);
    removeOpen = false;
  }

  function onWindowKey(e: KeyboardEvent) {
    if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
    const target = e.target as HTMLElement | null;
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

<svelte:window onkeydown={onWindowKey} />

<section class="screen">
  <StencilHeading level={1}>Mods</StencilHeading>
  <div class="layout">
    <div class="list-col">
      <FilterBar bind:text bind:chips {canMutate} onaddseparator={addSeparator} />
      {#if !ready}
        <div class="skeleton" aria-busy="true" aria-label="Loading mods">
          {#each [1, 2, 3, 4, 5] as n (n)}
            <Skeleton />
          {/each}
        </div>
      {:else if modItems.length === 0}
        <EmptyState title="No mods installed" body="Mods you install show up here in load order." />
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
          oneditseparator={editSeparator}
          onremoveseparator={askRemove}
        />
        {#if filtering && !visible.some((r) => r.kind === "mod")}
          <p class="qm-muted none">No mods match the filter.</p>
        {/if}
      {/if}
    </div>
    <aside class="detail" aria-label="Mod details">
      <DetailPanel
        mod={focusedMod}
        {canMutate}
        onenabledchange={(enabled) => focusedMod && void modList.setEnabled([focusedMod.id], enabled)}
      />
    </aside>
  </div>
</section>

<SeparatorDialog
  bind:open={editOpen}
  mode={editing ? "edit" : "add"}
  initial={editing ? { title: editing.title, color: editing.color } : { title: "", color: "sep-1" }}
  onsubmit={submitSeparator}
/>

<Dialog
  bind:open={removeOpen}
  title="Remove separator"
  description={removing ? `Remove "${removing.title}"? The mods under it stay where they are.` : ""}
>
  {#snippet footer()}
    <Button variant="ghost" onclick={() => (removeOpen = false)}>Cancel</Button>
    <Button variant="danger" onclick={confirmRemove}>Remove</Button>
  {/snippet}
</Dialog>

<style>
  .screen {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-4);
    padding: var(--qm-space-5);
  }

  .layout {
    display: flex;
    align-items: flex-start;
    gap: var(--qm-space-4);
  }

  .list-col {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--qm-space-3);
    min-width: 0;
  }

  .skeleton {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .none {
    margin: 0;
    font-size: var(--qm-text-13);
  }

  .detail {
    position: sticky;
    top: var(--qm-space-5);
    flex: none;
    width: var(--qm-detail-w);
  }
</style>
