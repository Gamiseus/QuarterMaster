<script lang="ts">
  import { Button, FieldRow, Panel, StencilHeading, Switch } from "../../components/ui";
  import type { SettingsPatch } from "../../lib/bindings";
  import { formatTimestamp } from "../../lib/saves/format";
  import { app } from "../../lib/stores/app.svelte";
  import { jobs } from "../../lib/stores/jobs.svelte";
  import { updates } from "../../lib/stores/updates.svelte";

  const s = $derived(app.settings);
  const disabled = $derived(!app.startupDone || !s);
  const checking = $derived(jobs.list.some((j) => j.kind === "updates_refresh"));
  const checkedAt = $derived(updates.report.status === "ready" ? updates.report.data.checked_at : "");

  function save(patch: Partial<SettingsPatch>) {
    app.updateSettings(patch).catch(() => {});
  }
</script>

<Panel class="settings-card">
  <StencilHeading level={2}>Updates</StencilHeading>
  <div>
    <FieldRow label="Check for updates automatically" help="At startup and every 6 hours while QuarterMaster is open.">
      <Switch
        label="Check for updates automatically"
        hideLabel
        {disabled}
        bind:checked={() => s?.auto_check_updates ?? true, (v) => save({ auto_check_updates: v })}
      />
    </FieldRow>
    <FieldRow
      label="Install mod updates automatically"
      help="One at a time, while the game is closed. A mod can override this in its details."
    >
      <Switch
        label="Install mod updates automatically"
        hideLabel
        {disabled}
        bind:checked={() => s?.auto_install_updates ?? false, (v) => save({ auto_install_updates: v })}
      />
    </FieldRow>
  </div>
  <div class="check">
    <Button size="sm" disabled={!app.startupDone || checking} onclick={() => updates.refresh().catch(() => {})}>Check now</Button>
    <span class="qm-muted when">Last checked: {checkedAt ? formatTimestamp(checkedAt) : "never"}</span>
  </div>
</Panel>

<style>
  .check {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-3);
    padding-top: var(--qm-space-3);
    border-top: 1px solid var(--qm-border);
  }

  .when {
    font-size: var(--qm-text-12);
  }
</style>
