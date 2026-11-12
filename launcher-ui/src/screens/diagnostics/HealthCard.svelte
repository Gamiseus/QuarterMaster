<script lang="ts">
  import { Badge, Button, ConfirmDialog, Inset, Panel, ScrollRegion, Skeleton, StencilHeading } from "../../components/ui";
  import type { RunEntry } from "../../lib/bindings";
  import { canRepairDll, canRestoreClean, healthRows, type HealthTone } from "../../lib/diagnostics/health";
  import { app } from "../../lib/stores/app.svelte";
  import { diagnostics } from "../../lib/stores/diagnostics.svelte";
  import { game } from "../../lib/stores/game.svelte";
  import { profiles } from "../../lib/stores/profiles.svelte";
  import SafeLaunchDialog from "./SafeLaunchDialog.svelte";

  const TONE_LABEL: Record<HealthTone, string> = { ok: "OK", warn: "Check", danger: "Problem", neutral: "Info" };
  /** Files named in the card and in the clear dialog; the rest are counted. */
  const SHOWN_FILES = 10;

  let repairOpen = $state(false);
  let restoreOpen = $state(false);
  let clearOpen = $state(false);
  let causeOpen = $state(false);
  /** The crashed run the dialog looks into; kept while a test run becomes the last run. */
  let causeRun = $state<RunEntry | null>(null);
  let acknowledging = $state(false);

  const report = $derived(diagnostics.health.status === "ready" ? diagnostics.health.data : null);
  const rows = $derived(
    report
      ? healthRows(report, game.isBusy, profiles.activeSummary?.id ?? null, (id) => profiles.list.find((p) => p.id === id)?.name)
      : [],
  );
  const canChange = $derived(app.startupDone && !game.isBusy);
  const canRepair = $derived(report !== null && app.startupDone && canRepairDll(report, game.isBusy));
  const canRestore = $derived(report !== null && app.startupDone && canRestoreClean(report, game.isBusy));
  const files = $derived(report?.overwrite_files ?? []);

  function findCause() {
    causeRun = report?.last_run ?? null;
    causeOpen = true;
  }

  async function acknowledge() {
    acknowledging = true;
    try {
      await diagnostics.acknowledgeUpdate();
    } catch {
      // call() already showed the banner or toast.
    } finally {
      acknowledging = false;
    }
  }
</script>

{#snippet fileList(label: string)}
  <ul class="files" aria-label={label}>
    {#each files.slice(0, SHOWN_FILES) as file (file)}<li>{file}</li>{/each}
    {#if files.length > SHOWN_FILES}
      <li class="qm-muted">and {files.length - SHOWN_FILES} more</li>
    {/if}
  </ul>
{/snippet}

<Panel class="health">
  <StencilHeading level={2}>Health</StencilHeading>
  {#if !report}
    <div aria-busy="true" aria-label="Checking the game folder"><Skeleton lines={5} /></div>
  {:else}
    <ScrollRegion class="card-scroll" label="Health details">
      <ul class="rows" aria-label="Health checks">
        {#each rows as row (row.label)}
          <li class="row">
            <span class="label">{row.label}</span>
            <Badge tone={row.tone}>{TONE_LABEL[row.tone]}</Badge>
            <div class="detail">
              <span>{row.detail}</span>
              {#if row.action === "clear_overwrite"}
                <Button size="sm" disabled={!canChange} onclick={() => (clearOpen = true)}>Clear overwrite folder</Button>
              {:else if row.action === "acknowledge_update"}
                <Button size="sm" disabled={!app.startupDone || acknowledging} onclick={acknowledge}>Mark as checked</Button>
              {:else if row.action === "find_cause"}
                <Button size="sm" disabled={!canChange} onclick={() => findCause()}>Find the cause</Button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
      {#if files.length > 0}
        <Inset>{@render fileList("Overwrite files")}</Inset>
      {/if}
    </ScrollRegion>
    <div class="actions">
      <Button disabled={!canRepair} onclick={() => (repairOpen = true)}>Repair DLL</Button>
      <Button disabled={!canRestore} onclick={() => (restoreOpen = true)}>Restore clean game folder</Button>
      {#if game.isBusy}<span class="qm-muted reason">Close the game to repair, restore or clear.</span>{/if}
    </div>
  {/if}
</Panel>

<SafeLaunchDialog bind:open={causeOpen} run={causeRun} />
<ConfirmDialog
  bind:open={repairOpen}
  title="Repair the hook DLL?"
  body="The winmm.dll in the game folder is replaced with the one that ships with this launcher."
  confirmLabel="Repair"
  onconfirm={() => diagnostics.repairDll()}
/>
<ConfirmDialog
  bind:open={restoreOpen}
  title="Restore the clean game folder?"
  body="QuarterMaster removes its files from the game folder. Your mods and saves are not touched."
  confirmLabel="Restore"
  onconfirm={() => diagnostics.restoreClean()}
/>
<ConfirmDialog
  bind:open={clearOpen}
  title="Clear the overwrite folder?"
  body="These files were written by the game or a mod into the active profile's Mods folder. They are deleted, not backed up."
  confirmLabel="Clear"
  onconfirm={() => diagnostics.clearOverwrite()}
>
  <Inset>{@render fileList("Files to delete")}</Inset>
</ConfirmDialog>

<style>
  .rows {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .row {
    display: grid;
    grid-template-columns: calc(var(--qm-detail-w) * 0.4) auto 1fr;
    gap: var(--qm-space-3);
    align-items: baseline;
    padding: var(--qm-space-2) 0;
    border-bottom: 1px solid var(--qm-border);
    font-size: var(--qm-text-13);
  }

  /* The actions' own top line closes the list, so the last row needs none. */
  .row:last-child {
    border-bottom: none;
  }

  .label {
    font-weight: 600;
  }

  .detail {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--qm-space-2);
  }

  .files {
    margin: 0;
    padding-left: var(--qm-space-4);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    overflow-wrap: anywhere;
  }

  .actions {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
    margin-top: var(--qm-space-3);
    padding-top: var(--qm-space-3);
    border-top: 1px solid var(--qm-border);
  }

  .reason {
    font-size: var(--qm-text-12);
  }
</style>
