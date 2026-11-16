<script lang="ts">
  import { Badge, Button, Callout, Dialog } from "../../components/ui";
  import type { ManualImportResult } from "../../lib/bindings";
  import { app } from "../../lib/stores/app.svelte";
  import { game } from "../../lib/stores/game.svelte";
  import { setup } from "../../lib/stores/setup.svelte";

  let busy = $state(false);
  let result = $state<ManualImportResult | null>(null);

  const found = $derived(setup.manual);
  const open = $derived((found !== null && !setup.manualDismissed) || result !== null);
  const fresh = $derived(found?.mods.filter((m) => m.installed_version === null) ?? []);
  const gameBusy = $derived(game.status.stage.stage !== "idle" && game.status.stage.stage !== "failed");

  async function run() {
    busy = true;
    try {
      result = await setup.importManual();
    } catch {
      // call() reported it; the dialog stays open.
    } finally {
      busy = false;
    }
  }

  function close() {
    if (busy) return;
    result = null;
    setup.closeManual();
  }
</script>

<Dialog
  bind:open={() => open, (v) => !v && close()}
  title={result ? "Manual install imported" : "Import your manual install?"}
  description={result
    ? undefined
    : "The game folder holds mods that were installed by hand. QuarterMaster can take them over, so you manage them here from now on."}
  size="lg"
  dismissible={!busy}
  explicitClose
>
  {#if result}
    <div class="body" data-testid="manual-result">
      <p class="line">
        {#if result.imported.length > 0}
          {result.imported.length === 1 ? "1 mod is" : `${result.imported.length} mods are`} now in the library{result.enabled
            ? " and switched on in the active profile"
            : ". The Vanilla profile runs no mods, so pick another profile to switch them on"}: {result.imported.join(", ")}.
        {:else}
          No mod was imported.
        {/if}
      </p>
      {#if result.unversioned.length > 0}
        <Callout tone="info" title="No version stated">
          {result.unversioned.join(", ")} {result.unversioned.length === 1 ? "states" : "state"} no version, so
          {result.unversioned.length === 1 ? "it was" : "they were"} imported as 0.0.0. Any version its author gives later
          counts as newer.
        </Callout>
      {/if}
      {#if result.skipped.length > 0}
        <Callout tone="warn" title="Left out">
          <ul class="plain">
            {#each result.skipped as s (s.folder)}
              <li><code>{s.folder}</code>: {s.reason}</li>
            {/each}
          </ul>
        </Callout>
      {/if}
      <p class="qm-muted line">
        {#if result.backup_dir}The old Mods folder was moved to the backups folder, with everything in it.{/if}
        {#if result.loader_removed}The old winmm.dll was deleted; QuarterMaster puts its own in place when you launch.{/if}
      </p>
    </div>
  {:else if found}
    <div class="body">
      {#if found.mods.length > 0}
        <ul class="mods" aria-label="Mods found in the game folder">
          {#each found.mods as m (m.folder)}
            <li>
              <span class="name">{m.name}</span>
              <span class="qm-muted version">{m.version_missing ? "no version" : m.version}</span>
              {#if m.installed_version !== null}
                <Badge tone="neutral">Already installed ({m.installed_version})</Badge>
              {:else if m.version_missing}
                <Badge tone="info">Imported as 0.0.0</Badge>
              {/if}
            </li>
          {/each}
        </ul>
      {:else if found.has_mods_folder}
        <p class="qm-muted line">The Mods folder holds no mods QuarterMaster can read.</p>
      {/if}
      {#if found.unreadable.length > 0}
        <Callout tone="warn" title="Cannot be read">
          <ul class="plain">
            {#each found.unreadable as u (u.folder)}
              <li><code>{u.folder}</code>: {u.reason}</li>
            {/each}
          </ul>
        </Callout>
      {/if}
      <Callout tone="info" title="What Import does">
        <ul class="plain">
          {#if fresh.length > 0}
            <li>Copies {fresh.length === 1 ? "the mod" : `the ${fresh.length} mods`} into the library and switches {fresh.length === 1 ? "it" : "them"} on in the active profile.</li>
          {/if}
          {#if found.has_mods_folder}
            <li>Moves the <code>Mods</code> folder out of the game folder into QuarterMaster's backups folder. Nothing in it is deleted.</li>
          {/if}
          {#if found.has_loader}
            <li><strong>Deletes the old <code>winmm.dll</code></strong> from the game folder. QuarterMaster puts its own in place each time you launch.</li>
          {/if}
        </ul>
      </Callout>
      {#if gameBusy}
        <p class="qm-muted line">Close the game to import.</p>
      {/if}
    </div>
  {/if}
  {#snippet footer()}
    {#if result}
      {#if result.backup_dir}
        <Button onclick={() => app.reveal("backups").catch(() => {})}>Open the backups folder</Button>
      {/if}
      <Button variant="primary" onclick={close}>Done</Button>
    {:else}
      <Button disabled={busy} onclick={close}>Not now</Button>
      <Button variant="primary" disabled={busy || gameBusy} onclick={run}>{busy ? "Importing..." : "Import"}</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .line {
    margin: 0;
    font-size: var(--qm-text-13);
  }

  .mods {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
  }

  .mods li {
    display: flex;
    align-items: center;
    gap: var(--qm-space-3);
    min-height: var(--qm-row-h);
    padding: 0 var(--qm-space-3);
  }

  .mods li + li {
    border-top: 1px solid var(--qm-border);
  }

  .name {
    font-weight: 600;
  }

  .version {
    margin-right: auto;
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  .plain {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    margin: 0;
    padding-left: var(--qm-space-4);
  }

  code {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }
</style>
