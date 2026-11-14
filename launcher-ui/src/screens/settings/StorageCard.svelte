<script lang="ts">
  import { Button, ConfirmDialog, Panel, StencilHeading } from "../../components/ui";
  import { call } from "../../lib/api";
  import { commands, type LeftoverBackup } from "../../lib/bindings";
  import { formatBytes } from "../../lib/saves/format";
  import { app } from "../../lib/stores/app.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";

  let copies = $state<LeftoverBackup[]>([]);
  let loaded = $state(false);
  let target = $state<LeftoverBackup | null>(null);
  let confirmOpen = $state(false);

  async function load() {
    try {
      copies = await call(commands.leftoverBackupsList());
    } catch {
      // call() showed the error.
    } finally {
      loaded = true;
    }
  }

  $effect(() => {
    if (app.startupDone) void load();
  });

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
    await load();
  }
</script>

<Panel class="settings-card">
  <StencilHeading level={2}>Copies of old Mods folders</StencilHeading>
  <p class="qm-muted note">
    When the game folder holds a real Mods folder that the launcher did not make, the launcher copies it here before
    putting its own in place, so nothing is lost. The copies are never used again; delete the ones you do not need.
  </p>
  {#if !loaded}
    <p class="qm-muted note" aria-busy="true">Looking for copies...</p>
  {:else if copies.length === 0}
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
  <div class="row">
    <Button size="sm" aria-label="Open the backups folder" onclick={() => app.reveal("backups").catch(() => {})}>Open folder</Button>
  </div>
</Panel>

<ConfirmDialog
  bind:open={confirmOpen}
  title="Delete this copy?"
  body={target ? `The copy from ${when(target)} (${formatBytes(target.size_bytes)}) is deleted for good. This cannot be undone.` : ""}
  confirmLabel="Delete"
  onconfirm={remove}
/>

<style>
  .note {
    margin: 0;
    font-size: var(--qm-text-12);
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

  .row {
    display: flex;
    gap: var(--qm-space-2);
  }
</style>
