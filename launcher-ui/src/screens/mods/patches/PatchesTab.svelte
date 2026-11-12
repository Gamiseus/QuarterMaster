<script lang="ts">
  import { untrack } from "svelte";
  import { Button, Callout, EmptyState, ScrollRegion, Select, Skeleton } from "../../../components/ui";
  import type { PatchExecutionRecord, StackMode } from "../../../lib/bindings";
  import { waitingNotes } from "../../../lib/diagnostics/health";
  import { modOfName } from "../../../lib/patches/compat";
  import { explainConflict, orderNotes, stackSentences, unlistedConflicts } from "../../../lib/patches/explain";
  import { STACK_MODES } from "../../../lib/patches/stacks";
  import { app } from "../../../lib/stores/app.svelte";
  import { loadOrder } from "../../../lib/stores/loadorder.svelte";
  import { mods } from "../../../lib/stores/mods.svelte";
  import { patches } from "../../../lib/stores/patches.svelte";
  import { patchView } from "../../../lib/stores/patchview.svelte";
  import { profiles } from "../../../lib/stores/profiles.svelte";
  import { lineRules } from "../../../lib/stores/linerules.svelte";
  import { stacking } from "../../../lib/stores/stacking.svelte";
  import AllOrder from "./AllOrder.svelte";
  import AssetsView from "./AssetsView.svelte";
  import DiffView from "./DiffView.svelte";
  import OrderView from "./OrderView.svelte";
  import FileList from "./FileList.svelte";
  import FixesView from "./FixesView.svelte";
  import PatchInspector from "./PatchInspector.svelte";
  import PatchStrip from "./PatchStrip.svelte";
  import LineRulesNotice from "./LineRulesNotice.svelte";
  import StackingNotice from "./StackingNotice.svelte";
  import StackModeDialog from "./StackModeDialog.svelte";
  import StackView from "./StackView.svelte";

  let modFilter = $state("");
  /** The stacking mode being previewed before Apply. */
  let modeDraft = $state<StackMode>("dependencies");
  let modeOpen = $state(false);
  /** Remounts the Select after a cancelled change, so it shows the mode in use again. */
  let modeKey = $state(0);

  const vanilla = $derived(profiles.activeSummary?.is_builtin ?? false);
  // While a write's new simulation runs, the last one stays on screen (and the views keep their state).
  const sim = $derived(loadOrder.shown);
  const stats = $derived(sim.status === "ready" ? sim.data.file_stats : []);
  /** Why patched files were not (all) compiled with the game's Lua, when they were not. */
  const compileNote = $derived(sim.status === "ready" ? sim.data.compile_check.note : null);
  /** The mods to look up a rung's patch in: the mod its id names, else every enabled mod. */
  const modsOf = (id: string) => {
    const mod = modOfName(id);
    return mod ? [mod] : (profiles.active?.enabled_mods ?? []);
  };
  /** A patch's name from its mod's patch list, else its id. */
  function patchName(id: string): string {
    for (const m of modsOf(id)) {
      const entry = patches.get(m);
      if (entry.status !== "ready") continue;
      const found = entry.data.find((p) => p.patch_id === id || `${m}:${p.patch_id}` === id);
      if (found?.name) return found.name;
    }
    return id;
  }
  /** Order rules and places the simulation does not use, each with the patch and why (G15). */
  const orderList = $derived(sim.status === "ready" ? orderNotes(sim.data, patchName) : []);
  // The patch lists the unused places need for their names.
  const rungMods = $derived(
    sim.status === "ready" ? [...new Set((sim.data.invalid_rungs ?? []).flatMap((r) => modsOf(r.patch_id)))] : [],
  );
  $effect(() => {
    const ids = rungMods;
    if (ids.length > 0) untrack(() => patches.loadAll(ids));
  });
  /** Adjusted copies the game skips while their mod is off (D19). */
  const waiting = $derived(sim.status === "ready" ? waitingNotes(sim.data.fix_bindings) : []);
  /** Conflicts no file lists (a module, a `[vars]` name): counted in the summary, so shown here. */
  const unlisted = $derived(sim.status === "ready" ? unlistedConflicts(sim.data.conflicts, stats) : []);
  const modNames = $derived(
    Object.fromEntries((mods.list.status === "ready" ? mods.list.data : []).map((m) => [m.id, m.name])),
  );
  const canMutate = $derived(app.startupDone);
  /** Views that use the patch inspector; the narrow layout hides it for the others. */
  const inspectorShown = $derived(
    patchView.view === "changes" || (patchView.view === "order" && patchView.orderScope === "file"),
  );
  const modName = (id: string) => modNames[id] ?? id;
  /** The selected patch's stacking notes in the shown file. */
  const stackNotes = $derived(
    patchView.patch && patchView.diff.status === "ready"
      ? stackSentences(patchView.patch, patchView.diff.data.result.stacks ?? [], modName)
      : [],
  );

  /** The selected patch's record in the shown file. */
  const selectedRecord = $derived(
    patchView.patch && patchView.diff.status === "ready"
      ? (patchView.diff.data.result.records.find((r) => r.patch_id === patchView.patch) ?? null)
      : null,
  );

  function pickMode(mode: StackMode) {
    if (mode === stacking.mode) return;
    modeDraft = mode;
    modeOpen = true;
  }

  // A closed dialog leaves the Select on the mode in use.
  $effect(() => {
    if (!modeOpen) untrack(() => modeKey++);
  });

  // While the stacking notice is unseen, check what dependency order would change.
  $effect(() => {
    void stacking.ensure();
  });

  function summary(records: PatchExecutionRecord[]): string {
    const applied = records.filter((r) => r.status === "applied").length;
    const broken = records.filter((r) => ["zero_matches", "count_mismatch", "aborted"].includes(r.status)).length;
    return `${applied} applied${broken > 0 ? `, ${broken} broken` : ""} in this file.`;
  }

  // Show the first file when nothing is picked yet, or when the picked file is no longer changed.
  $effect(() => {
    if (stats.length === 0) return;
    if (!patchView.file || !stats.some((s) => s.target_file === patchView.file)) {
      patchView.selectFile(stats[0].target_file);
    }
  });
</script>

{#snippet waitingList()}
  {#each waiting as note, i (`${note}:${i}`)}
    <p class="compile-note" role="note" data-testid="waiting-note">{note}</p>
  {/each}
  {#if unlisted.length > 0}
    <section class="unlisted" aria-label="Conflicts not in a file" data-testid="unlisted-conflicts">
      <p class="unlisted-title">{unlisted.length === 1 ? "1 conflict is" : `${unlisted.length} conflicts are`} not tied to a line in a file:</p>
      <ul>
        {#each unlisted as c (c.id)}
          <li>{explainConflict(c, { records: [], modName })}</li>
        {/each}
      </ul>
    </section>
  {/if}
{/snippet}

{#snippet views(scopes: boolean)}
  <div class="views" role="group" aria-label="View">
    <Button size="sm" variant={patchView.view === "changes" ? "secondary" : "ghost"} aria-pressed={patchView.view === "changes"} onclick={() => (patchView.view = "changes")}>Changes</Button>
    <Button size="sm" variant={patchView.view === "order" ? "secondary" : "ghost"} aria-pressed={patchView.view === "order"} onclick={() => (patchView.view = "order")}>Order</Button>
    <Button size="sm" variant={patchView.view === "fixes" ? "secondary" : "ghost"} aria-pressed={patchView.view === "fixes"} onclick={() => (patchView.view = "fixes")}>Fixes</Button>
    <Button size="sm" variant={patchView.view === "assets" ? "secondary" : "ghost"} aria-pressed={patchView.view === "assets"} onclick={() => (patchView.view = "assets")}>Assets</Button>
    {#if scopes && patchView.view === "order"}
      <span class="sep" aria-hidden="true"></span>
      <Button size="sm" variant={patchView.orderScope === "file" ? "secondary" : "ghost"} aria-pressed={patchView.orderScope === "file"} onclick={() => (patchView.orderScope = "file")}>This file</Button>
      <Button size="sm" variant={patchView.orderScope === "all" ? "secondary" : "ghost"} aria-pressed={patchView.orderScope === "all"} onclick={() => (patchView.orderScope = "all")}>All patches</Button>
      <Button size="sm" variant={patchView.orderScope === "stacks" ? "secondary" : "ghost"} aria-pressed={patchView.orderScope === "stacks"} onclick={() => (patchView.orderScope = "stacks")}>Shared lines</Button>
    {/if}
  </div>
{/snippet}

{#if vanilla}
  <EmptyState title="No patches" body="The Vanilla profile runs the game unmodded." />
{:else if sim.status !== "ready" || stats.length === 0}
  <!-- No file to show yet, but the fixes (Waiting ones too) stay reachable. -->
  <div class="patches-wrap bare">
    {@render views(false)}
    {#if patchView.view === "fixes"}
      {@render waitingList()}
      <ScrollRegion label="Fixes"><FixesView {modNames} {canMutate} /></ScrollRegion>
    {:else if patchView.view === "assets"}
      <ScrollRegion label="Assets"><AssetsView {modNames} {canMutate} /></ScrollRegion>
    {:else if sim.status === "loading"}
      <div aria-busy="true" aria-label="Loading patches"><Skeleton lines={6} /></div>
    {:else if sim.status === "error"}
      <Callout tone="danger" title="Could not load the patch simulation.">
        {sim.message}
        {#snippet action()}<Button size="sm" onclick={() => loadOrder.load().catch(() => {})}>Retry</Button>{/snippet}
      </Callout>
    {:else}
      {@render waitingList()}
      <EmptyState title="No changed files" body="No game files are changed by the enabled mods." />
    {/if}
  </div>
{:else}
  <div class="patches-wrap" aria-busy={loadOrder.refreshing}>
  <div class="pt-head">
    {#key modeKey}
      <Select
        label="Code at shared lines"
        items={STACK_MODES}
        bind:value={() => stacking.mode, (v) => pickMode(v)}
        disabled={!canMutate}
        class="stack-mode"
      />
    {/key}
  </div>
  {#if stacking.showNotice}
    <StackingNotice {modNames} {canMutate} />
  {/if}
  {#if lineRules.showNotice}
    <LineRulesNotice {canMutate} />
  {/if}
  {#if compileNote}
    <p class="compile-note" role="note" data-testid="compile-note">{compileNote}</p>
  {/if}
  {#if orderList.length > 0}
    <section class="unlisted" aria-label="Order notes" data-testid="order-notes">
      <p class="unlisted-title">{orderList.length === 1 ? "1 order rule or place is" : `${orderList.length} order rules or places are`} not used:</p>
      <ul>
        {#each orderList as note, i (`${note}:${i}`)}<li>{note}</li>{/each}
      </ul>
    </section>
  {/if}
  {@render waitingList()}
  <div class={["patches-tab", { "no-inspector": !inspectorShown }]}>
    <ScrollRegion label="Changed files" class="pt-files">
      <FileList {stats} selected={patchView.file} {modNames} bind:modFilter onselect={(f) => patchView.selectFile(f)} />
    </ScrollRegion>

    <section class="center" aria-label="File changes">
      {@render views(true)}
      {#if patchView.view === "fixes"}
        <ScrollRegion label="Fixes"><FixesView {modNames} {canMutate} /></ScrollRegion>
      {:else if patchView.view === "assets"}
        <ScrollRegion label="Assets"><AssetsView {modNames} {canMutate} /></ScrollRegion>
      {:else if patchView.view === "order" && patchView.orderScope === "all"}
        <ScrollRegion label="All patches in order"><AllOrder {stats} {modNames} /></ScrollRegion>
      {:else if patchView.view === "order" && patchView.orderScope === "stacks"}
        <ScrollRegion label="Shared lines"><StackView {modNames} {canMutate} /></ScrollRegion>
      {:else if patchView.diff.status === "loading"}
        <div aria-busy="true" aria-label="Loading the file"><Skeleton lines={8} /></div>
      {:else if patchView.diff.status === "error"}
        <Callout tone="danger" title="Could not load this file.">
          {patchView.diff.message}
          {#snippet action()}<Button size="sm" onclick={() => patchView.refresh()}>Retry</Button>{/snippet}
        </Callout>
      {:else}
        {@const diff = patchView.diff.data}
        {#if patchView.view === "order"}
          <ScrollRegion label="Patch order">
            <OrderView
              file={diff.result.target_file}
              patchIds={diff.result.records.map((r) => r.patch_id)}
              {modNames}
              {canMutate}
              summary={summary(diff.result.records)}
              edges={sim.status === "ready" ? (sim.data.inferred_edges ?? []).filter((e) => e.target_file === diff.result.target_file) : []}
            />
          </ScrollRegion>
        {:else}
        <PatchStrip
          records={diff.result.records}
          selected={patchView.patch}
          {modNames}
          colourMods={diff.result.mods_touching}
          onselect={(id) => patchView.selectPatch(id)}
        />
        <div class="diff"><DiffView {diff} {modNames} /></div>
        {/if}
      {/if}
    </section>

    <ScrollRegion label="Patch details" class="pt-inspector">
      {#if !patchView.detail}
        <p class="qm-muted">Pick a patch above the file to see what it does.</p>
      {:else if patchView.detail.status === "loading"}
        <div aria-busy="true" aria-label="Loading the patch"><Skeleton lines={5} /></div>
      {:else if patchView.detail.status === "error"}
        <Callout tone="danger" title="Could not load this patch.">
          {patchView.detail.message}
          {#snippet action()}<Button size="sm" onclick={() => patchView.refresh()}>Retry</Button>{/snippet}
        </Callout>
      {:else}
        <PatchInspector
          detail={patchView.detail.data}
          {modNames}
          {canMutate}
          {stackNotes}
          record={selectedRecord}
          onselect={(id) => patchView.openPatch(id)}
        />
      {/if}
    </ScrollRegion>
  </div>
  </div>
  <StackModeDialog bind:open={modeOpen} mode={modeDraft} {modNames} {canMutate} />
{/if}

<style>
  /* The tab measures itself: the Mods area is narrower than the window by the sidebar. */
  .patches-wrap {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }

  /* No file grid: the view switcher, then the state or the fixes. */
  .patches-wrap.bare {
    gap: var(--qm-space-3);
    padding-top: var(--qm-space-3);
  }

  .pt-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-3);
    padding-top: var(--qm-space-3);
  }

  /* Label and trigger on one row. */
  .pt-head :global(.stack-mode) {
    flex-direction: row;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .patches-wrap > :global(.stacking-notice) {
    margin-top: var(--qm-space-3);
  }

  .patches-tab {
    flex: 1;
    display: grid;
    grid-template-columns: minmax(190px, 240px) minmax(0, 1fr) minmax(260px, 340px);
    grid-template-areas: "files center inspector";
    gap: var(--qm-space-4);
    min-height: 0;
    padding-top: var(--qm-space-4);
  }

  .patches-tab :global(.pt-files) {
    grid-area: files;
  }

  .patches-tab :global(.pt-inspector) {
    grid-area: inspector;
  }

  /* Narrow: the inspector goes under the diff so the code keeps its width. */
  @container (max-width: 1080px) {
    .patches-tab {
      grid-template-columns: minmax(170px, 210px) minmax(0, 1fr);
      grid-template-rows: minmax(0, 3fr) minmax(0, 2fr);
      grid-template-areas:
        "files center"
        "files inspector";
    }

    /* Fixes, All patches and Shared lines do not use the inspector: the center takes its row. */
    .patches-tab.no-inspector {
      grid-template-rows: minmax(0, 1fr);
      grid-template-areas: "files center";
    }

    .patches-tab.no-inspector :global(.pt-inspector) {
      display: none;
    }
  }

  .center {
    grid-area: center;
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    min-width: 0;
    min-height: 0;
  }

  .views {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-1);
  }

  .sep {
    width: 1px;
    height: 20px;
    margin: 0 var(--qm-space-2);
    background: var(--qm-border);
  }

  .diff {
    flex: 1;
    min-height: 0;
  }

  p {
    margin: 0;
  }
  .compile-note {
    margin: 0 0 var(--qm-space-2);
    color: var(--qm-text-muted);
    font-size: var(--qm-text-13);
  }

  .unlisted {
    margin: 0 0 var(--qm-space-2);
    font-size: var(--qm-text-13);
  }

  .unlisted-title {
    font-weight: 600;
  }

  .unlisted ul {
    margin: var(--qm-space-1) 0 0;
    padding-left: var(--qm-space-4);
  }
</style>
