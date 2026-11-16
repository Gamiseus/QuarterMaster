<script lang="ts">
  import { untrack } from "svelte";
  import { Badge, Button, Callout, EmptyState, Section, Skeleton, Switch } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type FixSuggestion, type FixView, type PreviewResult, type RunAs } from "../../../lib/bindings";
  import { badgeOf, canReview, couldNotCheck, patchText, provenanceText, runLastText, runLikeText, sectionOf, statusNote } from "../../../lib/patches/fixlist";
  import { fixes, legacyRemoval, NOTICE_IMPORT } from "../../../lib/stores/fixes.svelte";
  import { patchView } from "../../../lib/stores/patchview.svelte";
  import { profiles } from "../../../lib/stores/profiles.svelte";
  import ExportCompatDialog from "./ExportCompatDialog.svelte";
  import FixReviewDialog from "./FixReviewDialog.svelte";
  import ImpactNotes from "./ImpactNotes.svelte";
  import RepickLineDialog from "./RepickLineDialog.svelte";
  import RunLikeDialog from "./RunLikeDialog.svelte";
  import UndoFixDialog from "./UndoFixDialog.svelte";

  interface Props {
    modNames: Record<string, string>;
    canMutate: boolean;
  }

  let { modNames, canMutate }: Props = $props();

  const view = $derived(fixes.view);
  const data = $derived(view.status === "ready" ? view.data : null);
  const list = $derived(data?.fixes ?? []);
  const attention = $derived(list.filter((f) => sectionOf(f) === "attention"));
  const working = $derived(list.filter((f) => sectionOf(f) === "working"));

  /** Suggestions put off with "Not now" (kept by the store, so a remount does not bring them back). */
  const later = $derived(fixes.later);
  const suggestions = $derived((data?.suggestions ?? []).filter((s) => !later.has(s.key)));
  let suggestionPreviews = $state<Record<string, PreviewResult | null>>({});
  /** Why a suggestion's last preview failed, by its key. */
  let previewErrors = $state<Record<string, string>>({});
  /** Bumped when a new answer lands: a preview of the old one no longer says what Apply does. */
  let previewGen = 0;
  $effect(() => {
    void data;
    untrack(() => {
      previewGen++;
      suggestionPreviews = {};
      previewErrors = {};
    });
  });

  /** Ids that tie each row's controls to its label, so a screen reader says which fix they are for. */
  const uid = $props.id();
  const labelId = (key: string) => `${uid}-${key.replace(/[^A-Za-z0-9_-]/g, (c) => `_${c.charCodeAt(0).toString(16)}`)}`;

  let importSeen = $state(false);
  /** How many earlier changes the import brought over, while its one-time notice is unseen. */
  const imported = $derived.by(() => {
    if (!data?.migrated || importSeen) return 0;
    if ((profiles.active?.notices_seen ?? []).includes(NOTICE_IMPORT)) return 0;
    return data.journal.filter((j) => j.op === "migrated").reduce((n, j) => n + j.added, 0);
  });

  /** Exports that can be undone, newest first (D71). */
  const exports = $derived(data?.migrated ? data.journal.filter((j) => j.op === "compat_export").reverse() : []);

  let undoFix = $state<FixView | null>(null);
  let undoOpen = $state(false);
  let reviewFix = $state<FixView | null>(null);
  let reviewOpen = $state(false);
  let repickFix = $state<FixView | null>(null);
  let repickOpen = $state(false);
  let runLikeFix = $state<FixView | null>(null);
  let runLikeOpen = $state(false);
  let runLikeAs = $state<RunAs>("base");
  let exportIds = $state<string[]>([]);
  let exportOpen = $state(false);
  let busy = $state(false);

  /** Kinds that can become a compatibility patch; an adjusted copy and a load order change cannot. */
  const EXPORTABLE = new Set(["disable", "order", "priority", "anchor", "stack_rule", "apply_as", "skip_hunk"]);

  function saveAsCompat(f: FixView) {
    exportIds = [f.id];
    exportOpen = true;
  }

  function askUndo(f: FixView) {
    undoFix = f;
    undoOpen = true;
  }

  function review(f: FixView) {
    reviewFix = f;
    reviewOpen = true;
  }

  function repick(f: FixView) {
    repickFix = f;
    repickOpen = true;
  }

  function runLike(f: FixView, as: RunAs = "base") {
    runLikeFix = f;
    runLikeAs = as;
    runLikeOpen = true;
  }

  function show(f: FixView) {
    if (!f.patch) return;
    patchView.view = "changes";
    void patchView.openPatch(f.patch.patch_id);
  }

  async function run(write: () => Promise<unknown>) {
    busy = true;
    try {
      await write();
    } catch {
      // call() already reported it.
    } finally {
      busy = false;
    }
  }

  function dismissImport() {
    importSeen = true;
    void run(() => fixes.dismissImportNotice());
  }

  async function previewSuggestion(s: FixSuggestion) {
    const gen = previewGen;
    suggestionPreviews = { ...suggestionPreviews, [s.key]: null };
    const errors = { ...previewErrors };
    delete errors[s.key];
    previewErrors = errors;
    let r: PreviewResult | undefined;
    let failed: string | null = null;
    try {
      r = await call(commands.simulationPreview({ add_fixes: s.drafts, remove_fixes: s.removes }), { expect: ["cancelled", "conflict"] });
    } catch (e) {
      // Replaced by a newer preview, or the simulation is still running: say so; Preview tries again.
      failed = couldNotCheck(e);
    }
    if (gen !== previewGen) return;
    if (r) suggestionPreviews = { ...suggestionPreviews, [s.key]: r };
    else {
      const next = { ...suggestionPreviews };
      delete next[s.key];
      suggestionPreviews = next;
      if (failed) previewErrors = { ...previewErrors, [s.key]: failed };
    }
  }

  /** One write: undoes `removes` (each alone) and makes `drafts`, as the preview checked. */
  async function applySuggestion(s: FixSuggestion) {
    await call(commands.fixSuggestionApply(s.drafts, s.removes));
    await fixes.load();
  }

  const name = (patchId: string) => {
    for (const f of list) for (const l of [f.patch, ...f.related]) if (l && l.patch_id === patchId) return l.name ?? patchId;
    return patchId;
  };
</script>

{#snippet row(f: FixView)}
  {@const badge = badgeOf(f)}
  {@const note = statusNote(f)}
  {@const named = labelId(`fix-${f.id}`)}
  {@const removal = legacyRemoval(f)}
  {@const like = runLikeText(f, modNames)}
  {@const last = runLastText(f)}
  <li>
    <article class="fix" aria-label={f.label}>
      <div class="top">
        <Badge tone={badge.tone}>{badge.text}</Badge>
        <span class="label" id={named}>{f.label}</span>
      </div>
      {#if f.target_file || f.patch}
        <p class="where">
          {#if f.target_file}<code>{f.target_file}</code>{/if}{#if f.target_file && f.patch}{" · "}{/if}{#if f.patch}{patchText(f.patch)}{/if}
        </p>
      {/if}
      <p class="qm-muted small">{provenanceText(f)}</p>
      {#if note}<p class="small">{note}</p>{/if}
      {#if f.fallback_text}<p class="small">{f.fallback_text}</p>{/if}
      <div class="actions">
        {#if !f.legacy}
          <Button size="sm" aria-describedby={named} disabled={!canMutate || busy} onclick={() => askUndo(f)}>Undo...</Button>
        {:else if removal}
          <!-- A row from before the fix list: taken out with the command that made it (D38). -->
          <Button size="sm" aria-describedby={named} disabled={!canMutate || busy} onclick={() => run(() => fixes.removeLegacy(f))}>{removal.text}</Button>
        {/if}
        {#if f.patch}
          <Button size="sm" variant="ghost" aria-describedby={named} onclick={() => show(f)}>Show in file</Button>
        {/if}
        {#if canReview(f)}
          <Button size="sm" aria-describedby={named} disabled={!canMutate || busy} onclick={() => review(f)}>Review...</Button>
        {/if}
        {#if like}
          <!-- An older copy runs after every mod's patches; this runs it as its base mod (D58). -->
          <Button size="sm" aria-describedby={named} disabled={!canMutate || busy} onclick={() => runLike(f)}>{like}...</Button>
        {:else if last}
          <!-- A copy switched to its mod can go back to running after every mod's patches. -->
          <Button size="sm" aria-describedby={named} disabled={!canMutate || busy} onclick={() => runLike(f, "legacy")}>{last}...</Button>
        {/if}
        <!-- A load order change cannot be paused; one paused before can still resume. -->
        {#if !f.legacy && (f.kind !== "mod_order" || f.paused)}
          <Switch
            label="Pause"
            aria-describedby={named}
            checked={f.paused}
            disabled={!canMutate || busy}
            onCheckedChange={(v: boolean) => run(() => fixes.setPaused(f.id, v))}
          />
        {/if}
        {#if !f.legacy && EXPORTABLE.has(f.kind)}
          <Button
            size="sm"
            variant="ghost"
            aria-describedby={named}
            disabled={!canMutate || busy || !f.can_export}
            title={f.export_blocked_reason ?? undefined}
            onclick={() => saveAsCompat(f)}>Save as compatibility patch...</Button
          >
        {/if}
      </div>
    </article>
  </li>
{/snippet}

<div class="fixes-view" aria-busy={fixes.refreshing}>
  {#if view.status === "loading"}
    <div aria-busy="true" aria-label="Loading fixes"><Skeleton lines={5} /></div>
  {:else if view.status === "error"}
    <Callout tone="danger" title="Could not load the fixes.">
      {view.message}
      {#snippet action()}<Button size="sm" onclick={() => fixes.load().catch(() => {})}>Retry</Button>{/snippet}
    </Callout>
  {:else if data}
    {#if imported > 0}
      <Callout tone="info">
        QuarterMaster now keeps a list of your fixes. {imported} earlier {imported === 1 ? "change was" : "changes were"} imported.
        {#snippet action()}<Button size="sm" disabled={busy} onclick={dismissImport}>Got it</Button>{/snippet}
      </Callout>
    {/if}
    {#if !data.migrated && data.migration_error}
      <Callout tone="warn" title="Your earlier fixes could not be imported yet.">
        {data.migration_error}
        {#snippet action()}
          <Button size="sm" disabled={!canMutate || busy} onclick={() => run(() => fixes.retryImport())}>Retry import</Button>
        {/snippet}
      </Callout>
    {/if}
    {#if !data.migrated && list.length > 0}
      <p class="qm-muted small">
        These are changes you made before QuarterMaster kept a list of fixes. Remove them here, or change them where you made them.
      </p>
    {/if}
    {#if data.unreadable_fixes > 0}
      <p class="qm-muted small">
        {data.unreadable_fixes} {data.unreadable_fixes === 1 ? "fix was" : "fixes were"} made by a newer QuarterMaster and {data.unreadable_fixes === 1 ? "is" : "are"} kept as {data.unreadable_fixes === 1 ? "it is" : "they are"}.
      </p>
    {/if}

    {#if data.plan_note}
      <p class="qm-muted small">{data.plan_note}</p>
    {/if}

    {#if data.suggestions_pending}
      <p class="qm-muted small" role="status">Checking for suggested fixes...</p>
    {:else if data.suggestions_note}
      <p class="qm-muted small">{data.suggestions_note}</p>
    {/if}

    {#if list.length === 0 && suggestions.length === 0}
      <EmptyState title="No fixes in this profile" body="Fixes you make in the conflict resolver or in patch order show here." />
    {:else}
      {#if attention.length > 0}
        <Section title="Needs your attention">
          <ul class="rows" aria-label="Fixes that need your attention">
            {#each attention as f (f.id)}{@render row(f)}{/each}
          </ul>
        </Section>
      {/if}
      {#if suggestions.length > 0}
        <Section title="Suggested">
          <ul class="rows" aria-label="Suggested fixes">
            {#each suggestions as s (s.key)}
              {@const p = suggestionPreviews[s.key]}
              {@const failed = previewErrors[s.key]}
              {@const about = s.fix_id ? list.find((f) => f.id === s.fix_id) : undefined}
              {@const named = labelId(`suggestion-${s.key}`)}
              <li>
                <article class="fix" aria-label={s.label}>
                  <div class="top">
                    <Badge tone="info">Suggested</Badge>
                    <span class="label" id={named}>{s.label}</span>
                  </div>
                  {#if p}<ImpactNotes impact={p.impact} {modNames} {name} />{/if}
                  {#if failed && p === undefined}<p class="small failed" role="alert">{failed}</p>{/if}
                  <div class="actions">
                    {#if s.kind === "rebind"}
                      <!-- Nothing to apply: the user checks the fix itself. -->
                      {#if about}
                        <Button size="sm" aria-describedby={named} disabled={!canMutate || busy} onclick={() => review(about)}>Review...</Button>
                      {/if}
                    {:else}
                      <Button size="sm" aria-describedby={named} disabled={p === null} onclick={() => previewSuggestion(s)}>{p === null ? "Checking..." : "Preview"}</Button>
                      <Button
                        size="sm"
                        variant="primary"
                        aria-describedby={named}
                        disabled={!canMutate || busy || !p || p.impact.breaks_file}
                        onclick={() => run(() => applySuggestion(s))}>Apply</Button
                      >
                    {/if}
                    <Button size="sm" variant="ghost" aria-describedby={named} onclick={() => fixes.putOff(s.key)}>Not now</Button>
                  </div>
                </article>
              </li>
            {/each}
          </ul>
        </Section>
      {/if}
      {#if working.length > 0}
        <Section title="Working">
          <ul class="rows" aria-label="Working fixes">
            {#each working as f (f.id)}{@render row(f)}{/each}
          </ul>
        </Section>
      {/if}
    {/if}
    {#if exports.length > 0}
      <Section title="Saved as compatibility patches">
        <ul class="rows" aria-label="Saved as compatibility patches">
          {#each exports as j (j.at)}
            {@const named = labelId(`step-${j.at}`)}
            <li>
              <article class="fix" aria-label={j.label}>
                <div class="top"><span class="label" id={named}>{j.label}</span></div>
                <p class="where qm-muted">{j.removed === 1 ? "1 fix" : `${j.removed} fixes`} moved into the mod.</p>
                <div class="actions">
                  <Button size="sm" aria-describedby={named} disabled={!canMutate || busy} onclick={() => run(() => fixes.askJournalUndo(j.at))}>Undo...</Button>
                </div>
              </article>
            </li>
          {/each}
        </ul>
      </Section>
    {/if}
  {/if}
</div>

<UndoFixDialog bind:open={undoOpen} fix={undoFix} {modNames} />
<FixReviewDialog bind:open={reviewOpen} fix={reviewFix} {canMutate} onundo={askUndo} onrepick={repick} />
<RepickLineDialog bind:open={repickOpen} fix={repickFix} {modNames} {canMutate} />
<RunLikeDialog bind:open={runLikeOpen} fix={runLikeFix} {modNames} runAs={runLikeAs} />
<ExportCompatDialog bind:open={exportOpen} fixIds={exportIds} {canMutate} />

<style>
  .fixes-view {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
    padding-bottom: var(--qm-space-3);
  }

  p {
    margin: 0;
  }

  .small {
    font-size: var(--qm-text-13);
  }

  .rows {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .fix {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    padding: var(--qm-space-2) var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-raised);
  }

  .top {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .label {
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .where {
    font-size: var(--qm-text-13);
    overflow-wrap: anywhere;
  }

  .where code {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  .failed {
    color: var(--qm-warn);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
    margin-top: var(--qm-space-1);
  }
</style>
