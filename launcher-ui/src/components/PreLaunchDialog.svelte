<script lang="ts">
  import type { LoadError } from "../lib/bindings";
  import { diagnosticKey, type FixAction } from "../lib/modlist/fixes";
  import { isModuleCompileError } from "../lib/patches/explain";
  import { foldConflicts } from "../lib/patches/groups";
  import { loadOrder } from "../lib/stores/loadorder.svelte";
  import { prelaunch } from "../lib/stores/prelaunch.svelte";
  import { mods } from "../lib/stores/mods.svelte";
  import { resolver } from "../lib/stores/resolver.svelte";
  import { lineRules } from "../lib/stores/linerules.svelte";
  import { stacking } from "../lib/stores/stacking.svelte";
  import { openFixes } from "../screens/mods/mods-view.svelte";
  import LineRulesNotice from "../screens/mods/patches/LineRulesNotice.svelte";
  import StackingNotice from "../screens/mods/patches/StackingNotice.svelte";
  import FailureGroupRow from "../screens/mods/patches/FailureGroupRow.svelte";
  import FixButton from "./FixButton.svelte";
  import { Button, Dialog } from "./ui";

  const errors = $derived(loadOrder.errors);
  const sim = $derived(loadOrder.simulation);
  /** Error conflicts (broken patches, files that do not compile), each with Resolve. */
  const broken = $derived(sim.status === "ready" ? sim.data.conflicts.filter((c) => c.severity === "error") : []);
  /** Failures caused by another patch fold under it (G10). */
  const brokenRows = $derived(foldConflicts(broken, sim.status === "ready" ? (sim.data.failure_groups ?? []) : []));

  function resolve(id: string) {
    prelaunch.open = false;
    resolver.show(id);
  }

  const modNames = $derived(Object.fromEntries((mods.list.status === "ready" ? mods.list.data : []).map((m) => [m.id, m.name])));
  const modName = (id: string) => modNames[id] ?? id;
  const blocked = $derived(prelaunch.blocked);
  const planError = $derived(prelaunch.planError);
  const loadErrors = $derived(prelaunch.loadErrors);
  const waiting = $derived(prelaunch.waiting);
  const fixWarnings = $derived(prelaunch.fixWarnings);

  const description = $derived(
    !blocked
      ? prelaunch.notice
        ? "Choose how code from several mods at one line is ordered before you play."
        : "Some of your fixes need a look. You can still launch."
      : planError !== null
      ? "QuarterMaster cannot read this profile's load order file, so the game would start with no mods."
      : prelaunch.compileErrors.length > 0
        ? prelaunch.compileErrors.every(isModuleCompileError)
          ? "A mod's module does not compile. The game would stop with a Lua error when it loads it."
          : "A patched file does not compile. The game would stop with a Lua error when it loads it."
        : prelaunch.abortedFiles.length > 0
          ? "A patch is set to stop when it cannot find its line. The game would stop with a Lua error when it loads that file."
          : loadErrors.length > 0 && errors.length === 0
            ? "Some mods could not be loaded. The game would start without them."
            : "The load order has errors. The game may crash or mods may not work.",
  );

  // The dialog only makes sense while something blocks a launch (it may clear while it is open),
  // while it shows the stacking notice, or while fixes need a look.
  $effect(() => {
    if (!prelaunch.blocked && !prelaunch.notice && prelaunch.fixWarnings.length === 0) prelaunch.open = false;
  });
  $effect(() => {
    if (!prelaunch.open) prelaunch.notice = false;
  });

  /** A fix that opened another view (the catalog, a mod's load order) closes the dialog to show it. */
  function fixed(action: FixAction) {
    if (action.kind === "browse" || action.kind === "open") prelaunch.open = false;
  }

  function launchAnyway() {
    const shown = prelaunch.notice;
    prelaunch.open = false;
    void prelaunch.launch(false, shown);
  }

  function reviewFixes() {
    prelaunch.open = false;
    openFixes();
  }

  function launchWithoutMods() {
    prelaunch.open = false;
    void prelaunch.launch(true);
  }

  /** "GCore: <why>", plus the mods skipped because they require it. */
  function loadErrorText(e: LoadError): string {
    const name = e.mod_id || e.folder;
    const also = e.skipped_dependents ?? [];
    const tail = also.length > 0 ? ` Also off because they need it: ${also.join(", ")}.` : "";
    return `${name}: ${e.message}.${tail}`;
  }
</script>

<Dialog
  bind:open={prelaunch.open}
  title={blocked ? "Fix before launching" : "Before you launch"}
  {description}
  initialFocus="content"
>
  {#if prelaunch.notice && stacking.unseen}
    <StackingNotice {modNames} canMutate={prelaunch.canLaunch} />
  {:else if prelaunch.notice && !blocked}
    <p class="note">Code at shared lines: {stacking.mode === "legacy" ? "older order" : stacking.mode === "load_order" ? "load order" : "dependencies first"}.</p>
  {/if}
  {#if prelaunch.notice && lineRules.showNotice}
    <LineRulesNotice canMutate={prelaunch.canLaunch} />
  {/if}
  {#if planError !== null}
    <p class="conflicts-title">Load order file</p>
    <ul class="errors">
      <li><span class="message">{planError}</span></li>
    </ul>
  {/if}
  {#if loadErrors.length > 0}
    <p class="conflicts-title">Mods that could not be loaded</p>
    <ul class="errors">
      {#each loadErrors as e, i (`${e.folder}:${i}`)}
        <li><span class="message">{loadErrorText(e)}</span></li>
      {/each}
    </ul>
    {#if planError === null}
      <p class="note">Launch anyway starts the game without these mods.</p>
    {/if}
  {/if}
  {#if errors.length > 0}
  <ul class="errors">
    {#each errors as d, i (`${diagnosticKey(d)}:${i}`)}
      <li>
        <div class="row">
          <span class="message">{d.message}</span>
          <FixButton diagnostic={d} onfixed={fixed} />
        </div>
      </li>
    {/each}
  </ul>
  {/if}
  {#if broken.length > 0}
    <p class="conflicts-title">Patch conflicts</p>
    <ul class="errors">
      {#each brokenRows as r (r.kind === "one" ? r.conflict.id : `group:${r.group.target_file}:${r.group.root}`)}
        <li>
          {#if r.kind === "one"}
            <div class="row">
              <span class="message">{r.conflict.description}</span>
              <Button size="sm" onclick={() => resolve(r.conflict.id)}>Resolve...</Button>
            </div>
          {:else}
            <FailureGroupRow row={r} {modName} onresolve={resolve} />
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if fixWarnings.length > 0}
    <p class="conflicts-title">Fixes</p>
    <div class="row">
      <ul class="notes" aria-label="Fixes to review">
        {#each fixWarnings as line (line)}<li>{line}</li>{/each}
      </ul>
      <Button size="sm" onclick={reviewFixes}>Review fixes</Button>
    </div>
  {/if}
  {#if waiting.length > 0}
    <p class="conflicts-title">Waiting</p>
    <ul class="notes" aria-label="Waiting adjusted copies">
      {#each waiting as note, i (`${note}:${i}`)}<li>{note}</li>{/each}
    </ul>
  {/if}
  {#snippet footer()}
    {#if !blocked}
      <Button variant="primary" disabled={!prelaunch.canLaunch} onclick={launchAnyway}>Launch</Button>
      <Button onclick={() => (prelaunch.open = false)}>Cancel</Button>
    {:else}
      {#if planError !== null}
        <Button disabled={!prelaunch.canLaunch} onclick={launchWithoutMods}>Launch without mods</Button>
      {:else}
        <Button disabled={!prelaunch.canLaunch} onclick={launchAnyway}>Launch anyway</Button>
      {/if}
      <Button variant="primary" onclick={() => (prelaunch.open = false)}>Cancel</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .errors {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: 0;
    padding: 0 0 0 var(--qm-space-4);
    color: var(--qm-danger);
  }

  .errors li {
    color: var(--qm-text);
  }

  /* A flex li would lose its list marker, so the row is an inner element. */
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-3);
  }

  .errors li::marker {
    color: var(--qm-danger);
  }
  .conflicts-title {
    margin: var(--qm-space-3) 0 var(--qm-space-1);
    font-weight: 600;
  }
  .notes {
    margin: 0;
    padding: 0 0 0 var(--qm-space-4);
    color: var(--qm-text-muted);
  }
  .note {
    margin: var(--qm-space-2) 0 0;
    color: var(--qm-text-muted);
  }
</style>
