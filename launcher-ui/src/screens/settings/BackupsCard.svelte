<script lang="ts">
  import { untrack } from "svelte";
  import { FieldRow, Panel, StencilHeading, Switch, TextField } from "../../components/ui";
  import { clampBackups } from "../../lib/settings/form";
  import { app } from "../../lib/stores/app.svelte";

  const s = $derived(app.settings);
  const disabled = $derived(!app.startupDone || !s);

  /** The number as typed; saved (clamped to 1..100) when the field loses focus. */
  let keep = $state("");

  $effect(() => {
    const saved = s?.max_save_backups;
    untrack(() => (keep = saved === undefined ? "" : String(saved)));
  });

  function commit() {
    if (!s) return;
    const n = clampBackups(Number(keep));
    keep = String(n);
    if (n !== s.max_save_backups) app.updateSettings({ max_save_backups: n }).catch(() => {});
  }
</script>

<Panel class="settings-card">
  <StencilHeading level={2}>Save backups</StencilHeading>
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
  </div>
</Panel>

<style>
  :global(.settings-card .number) {
    width: 10ch;
  }
</style>
