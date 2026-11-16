<script lang="ts">
  import { Button, ConfirmDialog, FieldRow, Panel, StencilHeading, Switch } from "../../components/ui";
  import type { SettingsPatch } from "../../lib/bindings";
  import { formatTimestamp } from "../../lib/saves/format";
  import { app } from "../../lib/stores/app.svelte";
  import { game } from "../../lib/stores/game.svelte";
  import { jobs } from "../../lib/stores/jobs.svelte";
  import { updates } from "../../lib/stores/updates.svelte";

  const s = $derived(app.settings);
  const disabled = $derived(!app.startupDone || !s);
  const checking = $derived(jobs.list.some((j) => j.kind === "updates_refresh"));
  const checkedAt = $derived(updates.report.status === "ready" ? updates.report.data.checked_at : "");

  const own = $derived(updates.launcher);
  /** Swapping the launcher closes it; not while the game or an install is using it. */
  const ownBlocked = $derived(
    !app.startupDone ? "Available once startup has finished." : game.isBusy ? "Close the game first." : updates.installing ? "Wait for the mod updates to finish." : null,
  );
  let rollbackOpen = $state(false);

  function save(patch: Partial<SettingsPatch>) {
    app.updateSettings(patch).catch(() => {});
  }
</script>

<Panel class="settings-card">
  <StencilHeading level={2}>Updates</StencilHeading>
  <div class="own" data-testid="launcher-update">
    <div class="own-text">
      <span class="own-name">QuarterMaster {own?.current_version ?? ""}</span>
      <span class="qm-muted own-state">
        {#if !own}
          Checking...
        {:else if own.update_available}
          Version {own.latest_version} is available. It replaces the launcher and the loader, then starts again.
        {:else if own.release_tag === ""}
          Not checked yet.
        {:else}
          Up to date.
        {/if}
      </span>
    </div>
    <div class="own-actions">
      {#if own?.update_available}
        <Button
          size="sm"
          variant="primary"
          disabled={ownBlocked !== null || updates.updatingLauncher}
          title={ownBlocked ?? undefined}
          onclick={() => updates.installLauncher().catch(() => {})}
        >
          {updates.updatingLauncher ? "Updating..." : "Update and restart"}
        </Button>
      {/if}
      {#if own?.rollback_available}
        <Button size="sm" variant="ghost" disabled={ownBlocked !== null || updates.updatingLauncher} title={ownBlocked ?? undefined} onclick={() => (rollbackOpen = true)}>
          Go back to the previous version
        </Button>
      {/if}
    </div>
  </div>
  <div>
    <FieldRow label="Check for updates automatically" help="QuarterMaster and your mods, at startup and every 6 hours while QuarterMaster is open.">
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
    <FieldRow
      label="Keep the old version when a mod updates"
      help="Off: the new version takes the old one's place. On: both stay installed, and other profiles stay on the old one."
    >
      <Switch
        label="Keep the old version when a mod updates"
        hideLabel
        {disabled}
        bind:checked={() => s?.keep_old_mod_versions ?? false, (v) => save({ keep_old_mod_versions: v })}
      />
    </FieldRow>
  </div>
  <div class="check">
    <Button size="sm" disabled={!app.startupDone || checking} onclick={() => updates.refresh().catch(() => {})}>Check now</Button>
    <span class="qm-muted when">Last checked: {checkedAt ? formatTimestamp(checkedAt) : "never"}</span>
  </div>
</Panel>

<ConfirmDialog
  bind:open={rollbackOpen}
  title="Go back to the previous version?"
  body="The launcher and loader that this version replaced are put back, and QuarterMaster starts again. Your mods, profiles and saves are not changed."
  confirmLabel="Go back and restart"
  danger={false}
  onconfirm={() => updates.rollbackLauncher()}
/>

<style>
  .own {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-3);
    padding-bottom: var(--qm-space-3);
    border-bottom: 1px solid var(--qm-border);
  }

  .own-text {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    min-width: 0;
  }

  .own-name {
    font-weight: 600;
  }

  .own-state {
    font-size: var(--qm-text-12);
  }

  .own-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-2);
  }

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
