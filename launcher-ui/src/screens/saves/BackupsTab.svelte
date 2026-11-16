<script lang="ts">
  import { Button, ConfirmDialog, EmptyState } from "../../components/ui";
  import type { SaveBackupEntry, SaveProfile } from "../../lib/bindings";
  import { formatBytes, formatTimestamp } from "../../lib/saves/format";
  import { saves } from "../../lib/stores/saves.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";
  import NameDialog from "../profiles/NameDialog.svelte";

  interface Props {
    profile: SaveProfile;
    /** Why a backup cannot be made now, or null. */
    backupBlocked: string | null;
    /** Why a backup cannot be restored now, or null. */
    restoreBlocked: string | null;
    /** Why a backup cannot be deleted now, or null. */
    deleteBlocked: string | null;
  }

  let { profile, backupBlocked, restoreBlocked, deleteBlocked }: Props = $props();

  let backupOpen = $state(false);
  let restoreOpen = $state(false);
  let deleteOpen = $state(false);
  let target = $state<SaveBackupEntry | null>(null);

  // Never render `path`: backups are referred to by label, time and size only.
  const backups = $derived([...(profile.backups ?? [])].sort((a, b) => b.timestamp.localeCompare(a.timestamp)));

  async function backUp(label: string) {
    await saves.backup(profile.id, label);
    toasts.info(`Backed up ${profile.name}.`);
  }

  async function restore() {
    if (!target) return;
    await saves.restore(profile.id, target.id);
    toasts.info(`Restored ${target.label}.`);
  }

  async function remove() {
    if (!target) return;
    await saves.deleteBackup(profile.id, target.id);
    toasts.info(`Deleted ${target.label}.`);
  }

  function ask(entry: SaveBackupEntry, which: "restore" | "delete") {
    target = entry;
    if (which === "restore") restoreOpen = true;
    else deleteOpen = true;
  }
</script>

<div class="backups">
  {#if backups.length === 0}
    <EmptyState title="No backups yet." body="A backup keeps a copy of this slot that you can restore later.">
      {#snippet action()}
        <Button variant="primary" disabled={backupBlocked !== null} onclick={() => (backupOpen = true)}>Back up now</Button>
      {/snippet}
    </EmptyState>
  {:else}
    <div class="bar">
      <Button variant="primary" disabled={backupBlocked !== null} onclick={() => (backupOpen = true)}>Back up now</Button>
      {#if backupBlocked}<span class="qm-muted reason">{backupBlocked}</span>{/if}
    </div>
    <ul class="list" aria-label="Backups">
      {#each backups as b (b.id)}
        <li class="row">
          <div class="info">
            <span class="label">{b.label}</span>
            <span class="qm-muted meta">{formatTimestamp(b.timestamp)}, {formatBytes(b.size_bytes)}</span>
          </div>
          <Button size="sm" disabled={restoreBlocked !== null} onclick={() => ask(b, "restore")}>
            Restore
          </Button>
          <Button size="sm" variant="ghost" disabled={deleteBlocked !== null} onclick={() => ask(b, "delete")}>Delete</Button>
        </li>
      {/each}
    </ul>
    {#if restoreBlocked}<p class="qm-muted reason">{restoreBlocked}</p>{/if}
  {/if}
</div>

<NameDialog
  bind:open={backupOpen}
  title="Back up {profile.name}"
  label="Label"
  submitLabel="Back up"
  initial="Manual backup"
  disabled={backupBlocked !== null}
  onsubmit={backUp}
/>
<ConfirmDialog
  bind:open={restoreOpen}
  title="Restore this backup?"
  body={target ? `The current save in ${profile.name} is replaced by "${target.label}".` : ""}
  confirmLabel="Restore"
  onconfirm={restore}
/>
<ConfirmDialog
  bind:open={deleteOpen}
  title="Delete this backup?"
  body={target ? `"${target.label}" is removed for good.` : ""}
  confirmLabel="Delete"
  onconfirm={remove}
/>

<style>
  .backups {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .bar {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .reason {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
    padding: var(--qm-space-2) 0;
    border-bottom: 1px solid var(--qm-border);
  }

  .info {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .label {
    font-size: var(--qm-text-14);
    overflow-wrap: anywhere;
  }

  .meta {
    font-size: var(--qm-text-12);
  }
</style>
