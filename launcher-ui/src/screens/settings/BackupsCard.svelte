<script lang="ts">
  import { untrack } from "svelte";
  import { Button, FieldRow, Panel, StencilHeading, Switch, TextField } from "../../components/ui";
  import { call } from "../../lib/api";
  import { commands, type LeftoverBackup } from "../../lib/bindings";
  import { formatBytes } from "../../lib/saves/format";
  import { clampBackups } from "../../lib/settings/form";
  import { app } from "../../lib/stores/app.svelte";
  import CopiesDialog from "./CopiesDialog.svelte";

  const s = $derived(app.settings);
  const disabled = $derived(!app.startupDone || !s);

  /** The number as typed; saved (clamped to 1..100) when the field loses focus. */
  let keep = $state("");

  $effect(() => {
    const saved = s?.max_save_backups;
    untrack(() => (keep = saved === undefined ? "" : String(saved)));
  });

  /** Copies the launcher made of real `Mods` folders it found in the game folder; their row shows only while there are some. */
  let copies = $state<LeftoverBackup[]>([]);
  let copiesOpen = $state(false);

  async function loadCopies() {
    try {
      copies = await call(commands.leftoverBackupsList());
    } catch {
      // call() showed the error.
    }
  }

  $effect(() => {
    if (app.startupDone) void loadCopies();
  });

  const total = $derived(copies.reduce((n, c) => n + c.size_bytes, 0));

  function commit() {
    if (!s) return;
    const n = clampBackups(Number(keep));
    keep = String(n);
    if (n !== s.max_save_backups) app.updateSettings({ max_save_backups: n }).catch(() => {});
  }
</script>

<Panel class="settings-card">
  <StencilHeading level={2}>Backups</StencilHeading>
  <div>
    <FieldRow label="Back up saves automatically" help="Before a slot switch or a restore.">
      <Switch
        label="Back up saves automatically"
        hideLabel
        {disabled}
        bind:checked={() => s?.auto_backup_saves ?? false, (v) => app.updateSettings({ auto_backup_saves: v }).catch(() => {})}
      />
    </FieldRow>
    <FieldRow
      label="Keep auto-backups"
      help="The oldest automatic backups are removed beyond this number. Manual backups are always kept."
    >
      <TextField
        class="number"
        label="Keep auto-backups"
        hideLabel
        type="number"
        min="1"
        max="100"
        {disabled}
        bind:value={keep}
        onblur={commit}
      />
    </FieldRow>
    <FieldRow label="Backups folder" help="Save backups, and the Mods folder of a manual install you imported.">
      <Button size="sm" aria-label="Open the backups folder" onclick={() => app.reveal("backups").catch(() => {})}>Open</Button>
    </FieldRow>
    {#if copies.length > 0}
      <div data-testid="leftover-row">
        <FieldRow
          label="Copies of old Mods folders"
          help="{copies.length === 1 ? '1 copy uses' : `${copies.length} copies use`} {formatBytes(total)}. They are never used again."
        >
          <Button size="sm" aria-label="Review the copies of old Mods folders" onclick={() => (copiesOpen = true)}>Review...</Button>
        </FieldRow>
      </div>
    {/if}
  </div>
</Panel>
<CopiesDialog bind:open={copiesOpen} {copies} onchanged={loadCopies} />

<style>
  :global(.settings-card .number) {
    width: 10ch;
  }
</style>
