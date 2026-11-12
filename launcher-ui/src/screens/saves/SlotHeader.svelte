<script lang="ts">
  import { IconMore } from "../../components/icons";
  import { Badge, Button, ConfirmDialog, IconButton, Menu, Select, StencilHeading } from "../../components/ui";
  import type { SaveSlotSummary, SaveSummary } from "../../lib/bindings";
  import { dossierText } from "../../lib/saves/dossier";
  import { slotTags } from "../../lib/saves/tags";
  import { app } from "../../lib/stores/app.svelte";
  import { game } from "../../lib/stores/game.svelte";
  import { profiles } from "../../lib/stores/profiles.svelte";
  import { saves } from "../../lib/stores/saves.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";
  import NameDialog from "../profiles/NameDialog.svelte";
  import { leftoverNote } from "../../lib/saves/leftover";
  import CleanupDialog from "./CleanupDialog.svelte";
  import CompareDialog from "./CompareDialog.svelte";

  interface Props {
    slot: SaveSlotSummary;
    /** The parsed save, for export; null while it loads. */
    summary: SaveSummary | null;
  }

  let { slot, summary }: Props = $props();

  const UNLOCK_WARNING = "Loading this save with mods marks it as modded for good.";
  const DELETE_ACTIVE = "Switch to another slot before deleting this one.";

  let renameOpen = $state(false);
  let cloneOpen = $state(false);
  let deleteOpen = $state(false);
  let unlockOpen = $state(false);
  let resetOpen = $state(false);
  let compareOpen = $state(false);
  let cleanupOpen = $state(false);
  let compareWith = $state<string | null>(null);
  let busy = $state(false);

  const p = $derived(slot.profile);
  const builtin = $derived(p.builtin ?? false);
  const startup = $derived(app.startupDone ? null : "Available once startup has finished.");
  const vanillaMismatch = $derived(
    (p.id === "vanilla") !== (profiles.activeSummary?.is_builtin ?? false)
      ? p.id === "vanilla"
        ? "The Vanilla save slot belongs to the Vanilla profile."
        : "The Vanilla profile always uses the Vanilla save slot."
      : null,
  );
  const switchBlocked = $derived(startup ?? (game.isBusy ? "Close the game to switch slots." : vanillaMismatch));
  const activeBusy = $derived(slot.is_active && game.isBusy);
  const backupBlocked = $derived(startup ?? (activeBusy ? "Close the game to back up the active slot." : null));
  const cloneBlocked = $derived(startup ?? (activeBusy ? "Close the game to clone the active slot." : null));
  const resetBlocked = $derived(startup ?? (game.isBusy ? "Close the game to reset a slot." : null));
  const cleanupBlocked = $derived(activeBusy ? "Close the game to clean up the active slot." : null);
  const deleteBlocked = $derived(startup ?? (slot.is_active ? DELETE_ACTIVE : null));
  const others = $derived(
    (saves.list ?? []).filter((s) => s.profile.id !== p.id).map((s) => ({ value: s.profile.id, label: s.profile.name })),
  );

  async function act(action: () => Promise<void>, done?: string) {
    if (busy) return;
    busy = true;
    try {
      await action();
      if (done) toasts.info(done);
    } catch {
      // call() already showed the banner or toast.
    } finally {
      busy = false;
    }
  }

  const switchSlot = () => act(() => saves.activate(p.id), `Switched to ${p.name}.`);
  const backUp = () => act(async () => void (await saves.backup(p.id, "Manual backup")), `Backed up ${p.name}.`);
  const lock = () => act(() => saves.setLock(p.id, true), `${p.name} is locked as vanilla.`);

  async function exportDossier() {
    if (!summary) return;
    await act(async () => {
      const file = await saves.exportDossier(p.name, dossierText(summary, p));
      if (file) toasts.info(`Saved the dossier as ${file}.`);
    });
  }

  function pickCompare(v: string | undefined) {
    if (!v) return;
    compareWith = v;
    compareOpen = true;
  }

  const more = $derived(
    builtin
      ? [
          { label: "Export dossier", disabled: summary === null, onselect: exportDossier },
          {
            label: "Reset to a new game...",
            separatorBefore: true,
            disabled: resetBlocked !== null,
            disabledReason: resetBlocked ?? undefined,
            onselect: () => (resetOpen = true),
          },
        ]
      : [
          { label: "Rename...", disabled: startup !== null, disabledReason: startup ?? undefined, onselect: () => (renameOpen = true) },
          { label: "Export dossier", disabled: summary === null, onselect: exportDossier },
          {
            label: "Reset to a new game...",
            disabled: resetBlocked !== null,
            disabledReason: resetBlocked ?? undefined,
            onselect: () => (resetOpen = true),
          },
          {
            label: "Delete...",
            separatorBefore: true,
            disabled: deleteBlocked !== null,
            disabledReason: deleteBlocked ?? undefined,
            onselect: () => (deleteOpen = true),
          },
        ],
  );
</script>

<header class="head">
  <div class="title">
    <StencilHeading level={2}>{p.name}</StencilHeading>
    {#if slot.is_active}<Badge tone="accent">Active</Badge>{/if}
    {#each slotTags(p) as tag (tag.label)}
      <Badge tone={tag.tone} icon={false}>{tag.label}</Badge>
    {/each}
  </div>
  {#if slot.tamper_note}
    <p class="note" role="note">{slot.tamper_note}</p>
  {/if}
  {#if slot.leftover}
    <div class="leftover" data-testid="save-leftover">
      <p class="note" role="note">{leftoverNote(slot.leftover)}</p>
      <Button size="sm" disabled={startup !== null} onclick={() => (cleanupOpen = true)}>
        {slot.leftover.cleanable.length > 0 ? "Clean up..." : "Show them"}
      </Button>
    </div>
  {/if}
  <div class="actions">
    <Button variant="primary" disabled={slot.is_active || switchBlocked !== null || busy} onclick={switchSlot}>
      Switch to this slot
    </Button>
    {#if builtin}
      <Button disabled={backupBlocked !== null || busy} onclick={backUp}>Back up now</Button>
    {:else}
      <Button disabled={cloneBlocked !== null} onclick={() => (cloneOpen = true)}>Clone...</Button>
      {#if p.locked_vanilla}
        <Button disabled={startup !== null || busy} onclick={() => (unlockOpen = true)}>Unlock</Button>
      {:else}
        <Button disabled={startup !== null || busy} onclick={lock}>Lock as vanilla</Button>
      {/if}
    {/if}
    {#if others.length > 0}
      <Select class="compare" label="Compare with" hideLabel placeholder="Compare with..." items={others} bind:value={() => undefined, pickCompare} />
    {/if}
    <Menu items={more} align="end">
      {#snippet trigger({ props })}
        <IconButton icon={IconMore} label="More slot actions" {...props} />
      {/snippet}
    </Menu>
  </div>
  {#if !slot.is_active && switchBlocked}
    <p class="qm-muted reason">{switchBlocked}</p>
  {/if}
  {#if builtin}
    <p class="qm-muted reason">The built-in slot for the Vanilla profile. It is always locked and cannot be renamed or deleted.</p>
  {/if}
</header>

<NameDialog
  bind:open={renameOpen}
  title="Rename {p.name}"
  label="Slot name"
  submitLabel="Rename"
  initial={p.name}
  onsubmit={(name) => act(() => saves.rename(p.id, name), "Slot renamed.")}
/>
<NameDialog
  bind:open={cloneOpen}
  title="Clone {p.name}"
  label="New slot name"
  submitLabel="Clone"
  disabled={cloneBlocked !== null}
  onsubmit={(name) => act(() => saves.clone(p.id, name), `Cloned to ${name}.`)}
/>
<ConfirmDialog
  bind:open={deleteOpen}
  title="Delete {p.name}?"
  body="The slot and its backups move to the deleted slots folder, where they are kept for 30 days."
  confirmLabel="Delete"
  onconfirm={() => saves.remove(p.id).then(() => toasts.info(`Deleted ${p.name}.`))}
/>
<ConfirmDialog
  bind:open={unlockOpen}
  title="Unlock {p.name}?"
  body={UNLOCK_WARNING}
  confirmLabel="Unlock"
  onconfirm={() => saves.setLock(p.id, false).then(() => toasts.info(`${p.name} is unlocked.`))}
/>
<ConfirmDialog
  bind:open={resetOpen}
  title="Reset {p.name} to a new game?"
  body={builtin
    ? "A backup is made first. The slot then starts a fresh vanilla game."
    : "A backup is made first. The slot then starts a fresh game; its tags stay."}
  confirmLabel="Reset"
  onconfirm={() => saves.resetSlot(p.id).then(() => toasts.info(`${p.name} starts a new game.`))}
/>
<CompareDialog bind:open={compareOpen} a={p.id} b={compareWith} />
{#if slot.leftover}
  <CleanupDialog bind:open={cleanupOpen} slotId={p.id} slotName={p.name} leftover={slot.leftover} blocked={cleanupBlocked} />
{/if}

<style>
  .head {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .title {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .actions :global(.compare) {
    width: calc(var(--qm-detail-w) * 0.6);
  }

  .reason,
  .note {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .note {
    color: var(--qm-warn);
  }

  .leftover {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }
</style>
