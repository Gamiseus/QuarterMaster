<script lang="ts">
  import { untrack } from "svelte";
  import { Button, Callout, Dialog, Section, Skeleton, Switch } from "../../../components/ui";
  import { call, QmCommandError } from "../../../lib/api";
  import {
    commands, type AnchorCandidate, type FileDiff, type PatchDetail, type PreviewChanges, type PreviewResult, type RegionOptions, type SiteKey,
  } from "../../../lib/bindings";
  import { dependentsToDisable } from "../../../lib/patches/cascade";
  import { explainConflict } from "../../../lib/patches/explain";
  import {
    canPickTarget, distinctOptions, edgeChange, fixOptions, fixPreview, lineCheck, moveChange, movesByCopy, previewGlue, regionFixOptions, retargetAnchors, rungPosition, withinChange,
    type AnchorChange, type AnchorPick, type FixAction, type FixOption as Fix,
  } from "../../../lib/patches/fixes";
  import { app } from "../../../lib/stores/app.svelte";
  import { fixes } from "../../../lib/stores/fixes.svelte";
  import { loadOrder } from "../../../lib/stores/loadorder.svelte";
  import { mods } from "../../../lib/stores/mods.svelte";
  import { NOT_IMPORTED, resolver, type UndoCheck } from "../../../lib/stores/resolver.svelte";
  import { openFixes } from "../mods-view.svelte";
  import AnchorPicker from "./AnchorPicker.svelte";
  import ConflictExplain from "./ConflictExplain.svelte";
  import FixOption from "./FixOption.svelte";

  type Loaded = {
    a?: PatchDetail;
    b?: PatchDetail;
    aDeps: PatchDetail[];
    bDeps: PatchDetail[];
    diff: FileDiff | null;
    candidates: AnchorCandidate[];
    /** Where the code can go inside the patch that took A's line (§9.2); null when not known. */
    region: RegionOptions | null;
    options: Fix[];
  };

  const sim = $derived(loadOrder.simulation);
  const conflict = $derived(
    sim.status === "ready" ? (sim.data.conflicts.find((c) => c.id === resolver.conflictId) ?? null) : null,
  );
  const applied = $derived(resolver.conflictId ? resolver.resolvedFor(resolver.conflictId) : undefined);
  const modNames = $derived({
    qm_glue: "Your adjustments",
    ...Object.fromEntries((mods.list.status === "ready" ? mods.list.data : []).map((m) => [m.id, m.name])),
  });
  const modName = (id: string) => modNames[id as keyof typeof modNames] ?? id;

  let loaded = $state<Loaded | null>(null);
  let previews = $state<Record<string, PreviewResult | null>>({});
  let showAll = $state(false);
  /** Which picker is open: a line anywhere in the file, or a line inside the other patch's change. */
  let picking = $state<"move" | "region" | null>(null);
  let busy = $state(false);
  /**
   * The conflict cannot be said to be gone yet: a write (Apply, Undo) clears the simulation until
   * the new one lands, and the fix that resolved it shows once the fixes reload.
   */
  const settling = $derived(!conflict && (sim.status === "loading" || busy || fixes.refreshing || fixes.view.status === "loading"));
  let error = $state<string | null>(null);
  /** Undo takes other fixes along or leaves changes alone: the user confirms first. */
  let undoAsk = $state<(UndoCheck & { conflictId: string }) | null>(null);
  let token = 0;

  const detail = (id: string) => call(commands.patchDetail(id), { expect: ["not_found", "conflict"] }).catch(() => undefined);
  /** The service dropped the preview for a newer one. */
  const cancelled = (e: unknown) => e instanceof QmCommandError && e.kind === "cancelled";
  const preview = (changes: PreviewChanges) => call(commands.simulationPreview(changes), { expect: ["cancelled", "conflict", "invalid_input"] });
  const name = (id: string) =>
    [loaded?.a, loaded?.b, ...(loaded?.aDeps ?? []), ...(loaded?.bDeps ?? [])].find((d) => d?.patch_id === id)?.name ?? id;

  /**
   * Previews a fix as the fixes it adds (§11.2). While the profile's earlier fixes cannot be
   * imported, falls back to the legacy change set when the fix has one (D38).
   */
  async function previewFix(action: FixAction, legacy: PreviewChanges | null, conflictId: string, focus: string): Promise<PreviewResult> {
    const drafts = fixPreview({ action }, conflictId, focus);
    if (!drafts) {
      if (!legacy) throw new Error("This fix cannot be checked.");
      return preview(legacy);
    }
    try {
      return await preview(drafts);
    } catch (e) {
      if (legacy && e instanceof QmCommandError && e.kind === "invalid_input" && e.message.startsWith(NOT_IMPORTED)) return preview(legacy);
      throw e;
    }
  }

  async function load(id: string) {
    const mine = ++token;
    loaded = null;
    previews = {};
    picking = null;
    error = null;
    const c = conflict;
    if (!c) return;
    const [a, b] = await Promise.all([detail(c.patch_a_id), c.patch_b_id ? detail(c.patch_b_id) : undefined]);
    const [aDeps, bDeps, diff, region] = await Promise.all([
      dependentsToDisable(c.patch_a_id, detail),
      c.patch_b_id ? dependentsToDisable(c.patch_b_id, detail) : Promise.resolve([]),
      call(commands.fileDiffGet(c.target_file), { expect: ["not_found"] })
        .then((r) => (r.status === "ready" ? r.data : null))
        .catch(() => null),
      canPickTarget(c, a)
        ? call(commands.regionOptions(c.target_file, c.patch_a_id, c.id), { expect: ["not_found", "conflict", "invalid_input"] }).catch(() => null)
        : Promise.resolve(null),
    ]);
    const records = diff?.result.records ?? [];
    const regionB = records.find((r) => r.patch_id === c.patch_b_id)?.regions[0];
    const line = regionB?.start_line ?? 1;
    const candidates = await call(commands.anchorCandidates(c.target_file, line, 8, null), { expect: ["not_found", "conflict"] }).catch(() => []);
    if (mine !== token || resolver.conflictId !== id) return;
    const options = distinctOptions([
      ...(region ? regionFixOptions(region, modName) : []),
      ...fixOptions(c, {
        a, b,
        aDependents: aDeps.map((d) => d.patch_id),
        bDependents: bDeps.map((d) => d.patch_id),
        retargets: retargetAnchors(c, records, candidates),
        name: (pid) => [a, b, ...aDeps, ...bDeps].find((d) => d?.patch_id === pid)?.name ?? pid,
        moveByCopy: region?.move_by_copy ?? movesByCopy(a),
        moveSites: moveSitesOf(region, c.consumer?.site_line ?? null),
      }),
    ]);
    loaded = { a, b, aDeps, bDeps, diff, candidates, region, options };
    // One preview at a time (the backend runs one per window); each fills in as it lands. Options
    // that cannot be used here were checked by the service already.
    for (const o of options) {
      if (o.unavailable) continue;
      const r = await previewFix(o.action, o.changes, c.id, c.target_file).catch((e) => {
        // A picker preview replaced it: not a failure.
        if (!cancelled(e)) error = e instanceof Error ? e.message : String(e);
        return null;
      });
      if (mine !== token) return;
      previews = { ...previews, [o.key]: r };
    }
  }

  /** The place a move takes when A has several: the one the conflict is about (D25). */
  function moveSitesOf(r: RegionOptions | null, line: number | null): SiteKey[] {
    const list = r?.a_sites ?? [];
    if (list.length <= 1) return [];
    return [(list.find((s) => s.anchor_line === line) ?? list[0]).key];
  }

  // Reload when a different conflict is shown, not on every simulation refresh (the conflict
  // object is new each time even when nothing about it changed).
  const shownId = $derived(resolver.open && conflict ? conflict.id : null);
  $effect(() => {
    if (shownId) void load(shownId);
  });

  const visible = $derived(
    (loaded?.options ?? []).filter(
      (o) => showAll || o.unavailable || previews[o.key] === undefined || previews[o.key] === null || previews[o.key]!.resolved.includes(conflict?.id ?? ""),
    ),
  );
  const hidden = $derived((loaded?.options.length ?? 0) - visible.length);
  /** "Pick a line inside <B>'s change...": a region option with no draft of its own. */
  const pickInside = $derived(loaded?.region?.options.find((o) => !o.draft) ?? null);
  const regionMod = $derived(loaded?.region?.b_mod_id ? modName(loaded.region.b_mod_id) : "");
  /** The patch replaces its line: it gets no place inside the other change (D6). */
  const atNote = $derived(
    loaded?.region?.a_is_at && loaded.region.consumer
      ? `"${name(loaded.region.patch_id)}" replaces its line, so it cannot go inside ${regionMod}'s change. Move to another line... makes an adjusted copy instead.`
      : null,
  );

  async function apply(action: Fix["action"], label: string, optionKey?: string, shown?: PreviewResult | null) {
    if (!conflict) return;
    busy = true;
    try {
      await resolver.apply(conflict.id, action, label, { optionKey, preview: shown ?? (optionKey ? previews[optionKey] : null) });
    } catch {
      // call() already reported it.
    } finally {
      busy = false;
    }
  }

  async function startUndo(conflictId: string) {
    busy = true;
    try {
      const check = await resolver.undoCheck(conflictId);
      if (check && (check.also.length > 0 || check.asks.length > 0)) undoAsk = { conflictId, ...check };
      else {
        // Not checked (the simulation is still running): the service says what needs a confirm.
        const ask = await resolver.undo(conflictId);
        if (ask) undoAsk = { conflictId, ...ask };
      }
    } catch {
      // call() already reported it.
    } finally {
      busy = false;
    }
  }

  async function confirmUndo() {
    const ask = undoAsk;
    if (!ask || ask.conflictId !== resolver.conflictId) {
      undoAsk = null;
      return;
    }
    busy = true;
    try {
      await resolver.undo(ask.conflictId, true);
      undoAsk = null;
    } catch {
      // call() already reported it.
    } finally {
      busy = false;
    }
  }

  function seeInFixes() {
    resolver.open = false;
    openFixes();
  }

  // A new conflict or a closed dialog drops a pending undo question.
  $effect(() => {
    const id = resolver.conflictId;
    const open = resolver.open;
    const ask = untrack(() => undoAsk);
    if (ask && (!open || ask.conflictId !== id)) undoAsk = null;
  });

  /** What a picked line writes, and its label. */
  async function pickAction(pick: AnchorPick): Promise<{ action: FixAction; label: string; legacy: PreviewChanges | null } | null> {
    const c = conflict;
    const l = loaded;
    if (!c || !l) return null;
    const a = c.patch_a_id;
    const nameA = name(a);
    const r = l.region;
    const sites = pick.aSite ? [pick.aSite] : (r?.options.find((o) => o.draft?.change.kind === "anchor")?.draft?.change as AnchorChange | undefined)?.sites ?? moveSitesOf(r, c.consumer?.site_line ?? null);
    if (pick.kind === "line") {
      if (r?.move_by_copy ?? movesByCopy(l.a)) {
        const glue = l.a ? previewGlue(l.a, pick.text.trim(), pick.position) : null;
        if (!glue) return null;
        return {
          action: { type: "glue", base: a, pattern: pick.text.trim(), position: pick.position },
          label: "Adjusted copy",
          legacy: { disable: [a], enable: [], order_rules: [], glue_patches: [glue], focus_file: c.target_file },
        };
      }
      return { action: { type: "anchor", change: moveChange(a, pick.text, rungPosition(l.a, pick.position), sites) }, label: `Move ${nameA} to another line`, legacy: null };
    }
    const b = r?.b_patch_id;
    if (!b) return null;
    if (pick.kind === "region") {
      return {
        action: { type: "anchor", change: withinChange(a, b, pick.site, pick.text, pick.nth, rungPosition(l.a, pick.position), sites) },
        label: `Put ${nameA} inside ${regionMod}'s change`,
        legacy: null,
      };
    }
    // The region option for the same edge already has the boundary check.
    const same = r.options.find((o) => {
      const rung = o.draft?.change.kind === "anchor" ? o.draft.change.anchors[0] : undefined;
      return rung?.edge === pick.edge && (rung.b_site ?? 0) === pick.site;
    });
    const change = same?.draft?.change.kind === "anchor" ? same.draft.change : edgeChange(a, b, pick.site, pick.edge, await lineCheck(pick.boundary), sites);
    return {
      action: { type: "anchor", change },
      label: same?.label ?? `Put it ${pick.edge === "start" ? "before" : "after"} ${regionMod}'s change`,
      legacy: null,
    };
  }

  let pickShown: PreviewResult | null = null;

  async function pickPreview(pick: AnchorPick) {
    const c = conflict;
    const made = await pickAction(pick);
    if (!c || !made) return null;
    const r = await previewFix(made.action, made.legacy, c.id, c.target_file).catch((e) => {
      // A newer pick replaced this preview: the picker drops it.
      if (cancelled(e)) throw e;
      error = e instanceof Error ? e.message : String(e);
      return null;
    });
    pickShown = r;
    return r;
  }

  async function pickApply(pick: AnchorPick) {
    const made = await pickAction(pick);
    if (made) await apply(made.action, made.label, undefined, pickShown);
  }

  function only(keep: "a" | "b") {
    if (!conflict?.patch_b_id) return Promise.resolve(null);
    const off = keep === "a" ? conflict.patch_b_id : conflict.patch_a_id;
    return preview({ disable: [off], enable: [], order_rules: [], glue_patches: [], focus_file: conflict.target_file }).catch(() => null);
  }
</script>


<Dialog bind:open={resolver.open} title="Resolve conflict" size="lg" initialFocus="content" dismissible={!busy}>
  {#if applied}
    <div class="applied" role="status">
      <p>Resolved by: {applied.label}</p>
      <Button size="sm" onclick={seeInFixes}>See in Fixes</Button>
      <Button size="sm" disabled={busy || !app.startupDone || undoAsk !== null} onclick={() => startUndo(applied.conflictId)}>Undo</Button>
    </div>
    {#if undoAsk}
      <Callout tone="warn" title="Undo changes more than this fix.">
        <ul class="asks" aria-label="What undo also does">
          {#each undoAsk.also as label (label)}<li>Also undoes: {label}</li>{/each}
          {#each undoAsk.asks as ask (ask)}<li>{ask}</li>{/each}
        </ul>
        {#snippet action()}
          <div class="ask-actions">
            <Button size="sm" disabled={busy} onclick={() => (undoAsk = null)}>Cancel</Button>
            <Button size="sm" variant="danger" disabled={busy || !app.startupDone} onclick={confirmUndo}>Undo anyway</Button>
          </div>
        {/snippet}
      </Callout>
    {/if}
  {:else if settling}
    <div aria-busy="true" aria-label="Checking the conflict"><Skeleton lines={3} /></div>
  {:else if !conflict && sim.status === "error"}
    <p class="qm-muted">Could not check this conflict: the patch simulation did not load.</p>
  {:else if !conflict}
    <p class="qm-muted">This conflict is gone. The simulation no longer reports it.</p>
  {:else if !loaded}
    <div aria-busy="true" aria-label="Loading the conflict"><Skeleton lines={6} /></div>
  {:else}
    <div class="body">
      <ConflictExplain
        {conflict}
        sentence={explainConflict(conflict, { a: loaded.a, b: loaded.b, records: loaded.diff?.result.records ?? [], modName })}
        a={loaded.a}
        b={loaded.b}
        aDependents={loaded.aDeps.map((d) => d.patch_id)}
        bDependents={loaded.bDeps.map((d) => d.patch_id)}
        records={loaded.diff?.result.records ?? []}
        diff={loaded.diff}
        {modNames}
        {only}
      />
      {#if error}<Callout tone="warn">Some fixes could not be checked: {error}</Callout>{/if}
      {#if atNote}<p class="qm-muted">{atNote}</p>{/if}
      <Section title="Fixes">
        {#snippet actions()}<Switch label="Show all options" bind:checked={showAll} />{/snippet}
        {#each visible as o (o.key)}
          <FixOption
            option={o}
            conflictId={conflict.id}
            preview={previews[o.key] ?? null}
            original={loaded.diff?.original_lines ?? []}
            {modNames}
            {name}
            canMutate={app.startupDone && !busy}
            onapply={() => apply(o.action, o.label, o.key)}
          />
        {/each}
        {#if hidden > 0 && !showAll}
          <p class="qm-muted">{hidden} {hidden === 1 ? "option does" : "options do"} not resolve this conflict and {hidden === 1 ? "is" : "are"} hidden.</p>
        {/if}
        {#if pickInside || canPickTarget(conflict, loaded.a)}
          <div class="pick">
            <div class="pick-buttons">
              {#if pickInside && loaded.region}
                <Button size="sm" aria-expanded={picking === "region"} onclick={() => (picking = picking === "region" ? null : "region")}>{pickInside.label}</Button>
              {/if}
              {#if canPickTarget(conflict, loaded.a)}
                <Button size="sm" aria-expanded={picking === "move"} onclick={() => (picking = picking === "move" ? null : "move")}>Move to another line...</Button>
              {/if}
            </div>
            {#if picking === "region" && loaded.region?.b_patch_id}
                <AnchorPicker
                  candidates={[]}
                  region={{ patchId: loaded.region.b_patch_id, modName: regionMod, patchName: name(loaded.region.b_patch_id), sites: loaded.region.sites }}
                  conflictId={conflict.id}
                  original={loaded.diff?.original_lines ?? []}
                  {modNames}
                  canMutate={app.startupDone && !busy}
                  onpreview={pickPreview}
                  onapply={pickApply}
                />
            {:else if picking === "move"}
                <AnchorPicker
                  candidates={loaded.candidates}
                  aSites={loaded.region?.move_by_copy === false ? loaded.region.a_sites : []}
                  aSiteLine={conflict.consumer?.site_line ?? null}
                  conflictId={conflict.id}
                  original={loaded.diff?.original_lines ?? []}
                  {modNames}
                  canMutate={app.startupDone && !busy}
                  onpreview={pickPreview}
                  onapply={pickApply}
                />
            {/if}
          </div>
        {/if}
      </Section>
    </div>
  {/if}
  {#snippet footer()}
    <Button variant="primary" disabled={busy} onclick={() => (resolver.open = false)}>Close</Button>
  {/snippet}
</Dialog>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-4);
  }

  .applied {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-3);
  }

  .applied p {
    flex: 1;
  }

  .asks {
    margin: 0;
    padding: 0 0 0 var(--qm-space-4);
  }

  .ask-actions {
    display: flex;
    gap: var(--qm-space-2);
  }

  p {
    margin: 0;
  }

  .pick {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    padding-top: var(--qm-space-2);
  }

  .pick-buttons {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-2);
  }
</style>
