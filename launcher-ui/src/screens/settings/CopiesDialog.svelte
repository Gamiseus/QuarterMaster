<script lang="ts">
  import { Button, ConfirmDialog, Dialog } from "../../components/ui";
  import { call } from "../../lib/api";
  import { commands, type LeftoverBackup } from "../../lib/bindings";
  import { formatBytes } from "../../lib/saves/format";
  import { app } from "../../lib/stores/app.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";

  interface Props {
    open?: boolean;
    copies: LeftoverBackup[];
    /** Called after a copy was deleted, to read the list again. */
    onchanged: () => Promise<void>;
  }

  let { open = $bindable(false), copies, onchanged }: Props = $props();

  let target = $state<LeftoverBackup | null>(null);
  let confirmOpen = $state(false);

  const total = $derived(copies.reduce((n, c) => n + c.size_bytes, 0));

  /** `unlinked-mods-20261102-181455` -> a local date and time. */
  function when(c: LeftoverBackup): string {
    const m = /(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})(\d{2})$/.exec(c.name);
    const d = m ? new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]) : c.modified ? new Date(c.modified * 1000) : null;
    if (!d || Number.isNaN(d.getTime())) return c.name;
    return d.toLocaleString(undefined, { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });
  }

  function ask(c: LeftoverBackup) {
    target = c;
    confirmOpen = true;
  }

  async function remove() {
    if (!target) return;
    const gone = target;
    await call(commands.leftoverBackupDelete(gone.name));
    toasts.info(`Deleted the copy from ${when(gone)} (${formatBytes(gone.size_bytes)}).`);
    await onchanged();
  }
</script>

<Dialog
  bind:open
  title="Copies of old Mods folders"
  description="When the game folder held a real Mods folder that the launcher did not make, the launcher copied it to its backups folder before putting its own in place. The copies are never used again; delete the ones you do not need."
  size="lg"
>
  {#if copies.length === 0}
    <p class="note" data-testid="no-leftovers">There are no copies.</p>
  {:else}
    <p class="note" data-testid="leftover-total">
      {copies.length === 1 ? "1 copy uses" : `${copies.length} copies use`} {formatBytes(total)}.
    </p>
    <ul class="copies" aria-label="Copies of old Mods folders">
      {#each copies as c (c.name)}
        <li data-testid="leftover-backup">
          <div class="what">
            <span class="when">{when(c)}</span>
            <span class="qm-muted meta">{formatBytes(c.size_bytes)}, {c.file_count} {c.file_count === 1 ? "file" : "files"}</span>
            {#if c.contents.length > 0}
              <span class="qm-muted meta in">Holds: {c.contents.join(", ")}</span>
            {/if}
          </div>
          <Button size="sm" variant="danger" aria-label={`Delete the copy from ${when(c)}`} onclick={() => ask(c)}>Delete</Button>
        </li>
      {/each}
    </ul>
  {/if}
  {#snippet footer()}
    <Button onclick={() => app.reveal("backups").catch(() => {})}>Open the backups folder</Button>
    <Button variant="primary" onclick={() => (open = false)}>Done</Button>
  {/snippet}
</Dialog>

<ConfirmDialog
  bind:open={confirmOpen}
  title="Delete this copy?"
  body={target ? `The copy from ${when(target)} (${formatBytes(target.size_bytes)}) is deleted for good. This cannot be undone.` : ""}
  confirmLabel="Delete"
  onconfirm={remove}
/>

<style>
  .note {
    margin: 0 0 var(--qm-space-2);
    font-size: var(--qm-text-13);
  }

  .copies {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .copies li {
    display: flex;
    align-items: center;
    gap: var(--qm-space-3);
    padding: var(--qm-space-2) 0;
    border-top: 1px solid var(--qm-border);
  }

  .what {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .when {
    font-size: var(--qm-text-13);
  }

  .meta {
    font-size: var(--qm-text-12);
  }

  .in {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

</style>
