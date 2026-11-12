<script lang="ts">
  import { Badge, Button, Callout, Checkbox } from "../../../components/ui";
  import type { FileDiff, PreviewResult } from "../../../lib/bindings";
  import { fallbackLine } from "../../../lib/patches/explain";
  import type { FixOption } from "../../../lib/patches/fixes";
  import { movedBlocks, outcomeChanges, replacedFixes } from "../../../lib/patches/impact";
  import { fixes } from "../../../lib/stores/fixes.svelte";
  import DiffView from "./DiffView.svelte";
  import FactionNote from "./FactionNote.svelte";

  interface Props {
    option: FixOption;
    conflictId: string;
    /** null while its preview runs. */
    preview: PreviewResult | null;
    original: string[];
    modNames: Record<string, string>;
    name: (patchId: string) => string;
    canMutate: boolean;
    onapply: () => void;
  }

  let { option, conflictId, preview, original, modNames, name, canMutate, onapply }: Props = $props();

  let expanded = $state(false);
  let understood = $state(false);
  const unavailable = $derived(option.unavailable ?? null);
  const resolves = $derived(preview ? preview.resolved.includes(conflictId) : null);
  const impact = $derived(preview?.impact ?? null);
  /** The fix would stop a file from compiling: it is shown but cannot be applied. */
  const breaks = $derived(impact?.breaks_file ?? false);
  /** Other patches whose outcome the fix changes, in words. */
  const sideEffects = $derived(outcomeChanges(impact, name, option.disables));
  const moved = $derived(movedBlocks(impact));
  const fallbacks = $derived(impact?.fallbacks_new ?? []);
  const fixedFallbacks = $derived(impact?.fallbacks_fixed ?? []);
  /** The user's fixes this one takes the place of. */
  const replaced = $derived(replacedFixes(impact, (id) => fixes.list.find((f) => f.id === id)?.label ?? null));
  /** An order that runs a mod before one it depends on: Apply needs "I understand" (D39). */
  const warnings = $derived(option.action.type === "order" ? (impact?.relation_warnings ?? []) : []);
  const diff = $derived<FileDiff | null>(preview?.file ? { original_lines: original, result: preview.file } : null);
  const uid = $props.id();
</script>

<div class={["fix", { unavailable }]}>
  <div class="head">
    <button type="button" class="toggle" aria-expanded={expanded} aria-controls={`${uid}-body`} onclick={() => (expanded = !expanded)}>
      {option.label}
    </button>
    {#if unavailable}
      <Badge>Cannot be used</Badge>
    {:else if resolves === null}
      <Badge>Checking...</Badge>
    {:else if breaks}
      <Badge tone="danger">Breaks {impact?.compile_errors_new.join(", ")}</Badge>
    {:else if resolves}
      <Badge tone="ok">Resolves this conflict</Badge>
    {:else}
      <Badge tone="warn">Does not resolve it</Badge>
    {/if}
    {#if preview && preview.new_conflicts.length > 0}
      <Badge tone="warn">{preview.new_conflicts.length} new {preview.new_conflicts.length === 1 ? "conflict" : "conflicts"}</Badge>
    {/if}
    {#if moved > 0}
      <Badge tone="info">Moves {moved} shared-line {moved === 1 ? "block" : "blocks"}</Badge>
    {/if}
    {#if fallbacks.length > 0}
      <Badge tone="warn">Uses its own line elsewhere</Badge>
    {/if}
    <Button
      size="sm"
      disabled={!canMutate || unavailable !== null || preview === null || breaks || (warnings.length > 0 && !understood)}
      title={breaks ? "This fix would stop a game file from compiling." : undefined}
      onclick={onapply}>Apply</Button
    >
  </div>
  {#if unavailable}
    <p class="qm-muted why">Cannot be used: {unavailable}.</p>
  {/if}
  {#if option.note}<p class="qm-muted">{option.note}.</p>{/if}
  {#if option.oldText && option.newText}
    <dl class="lines">
      <dt>Was</dt><dd><code>{option.oldText.trim()}</code></dd>
      <dt>Now</dt><dd><code>{option.newText.trim()}</code></dd>
    </dl>
  {/if}
  {#if option.nextTo && !unavailable}<p class="qm-muted">{fallbackLine(option.nextTo)}</p>{/if}
  {#if warnings.length > 0}
    <Callout tone="warn" title="This order may break a mod.">
      <ul class="warnings">
        {#each warnings as w (w.rule + w.text)}<li>{w.text}.</li>{/each}
      </ul>
      {#snippet action()}<Checkbox label="I understand" bind:checked={understood} />{/snippet}
    </Callout>
  {/if}
  <FactionNote {impact} {modNames} />
  {#if expanded}
    <div class="body" id={`${uid}-body`}>
      {#if option.disables.length > 0}
        <p>Turns off for this profile: {option.disables.map(name).join(", ")}.</p>
      {/if}
      {#if preview && preview.new_conflicts.length > 0}
        <p>Causes: {preview.conflicts.filter((c) => preview.new_conflicts.includes(c.id)).map((c) => c.description).join(" ")}</p>
      {/if}
      {#if sideEffects.length > 0}
        <p>Also: {sideEffects.join("; ")}.</p>
      {/if}
      {#each fallbacks as f (f.file + f.patch_id + f.site_line)}
        <p>{name(f.patch_id)} would use its own line in {f.file}: {f.reason}.</p>
      {/each}
      {#each replaced as label (label)}
        <p>Replaces your fix "{label}".</p>
      {/each}
      {#each fixedFallbacks as f (f.file + f.patch_id + f.site_line)}
        <p>{name(f.patch_id)} goes back to the line its fix puts it on in {f.file}.</p>
      {/each}
      {#if impact && impact.compile_errors_fixed.length > 0}
        <p>Makes {impact.compile_errors_fixed.join(", ")} compile again.</p>
      {/if}
      {#if impact && impact.compile_errors_existing.length > 0}
        <p class="qm-muted">{impact.compile_errors_existing.join(", ")} still does not compile.</p>
      {/if}
      {#if impact && impact.files_changed.length > 1}
        <p class="qm-muted">Changes {impact.files_changed.length} files: {impact.files_changed.join(", ")}.</p>
      {/if}
      {#if diff}
        <div class="preview"><DiffView {diff} {modNames} /></div>
      {:else if preview}
        <p class="qm-muted">The file has no patches left after this fix.</p>
      {:else if unavailable}
        <p class="qm-muted">No preview: this option cannot be used here.</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .fix {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    border-bottom: 1px solid var(--qm-border);
    padding: var(--qm-space-2) 0;
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .toggle {
    flex: 1;
    min-width: 12em;
    padding: 0;
    border: 0;
    background: none;
    color: var(--qm-text);
    font: inherit;
    font-weight: 600;
    text-align: left;
    cursor: pointer;
  }

  .unavailable .toggle {
    color: var(--qm-text-muted);
  }

  .toggle::before {
    content: "+ ";
    color: var(--qm-text-muted);
  }

  .toggle[aria-expanded="true"]::before {
    content: "- ";
  }

  .toggle:focus-visible {
    outline: none;
    box-shadow: var(--qm-focus-ring);
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    padding-top: var(--qm-space-2);
  }

  p {
    margin: 0;
  }

  .lines {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 0 var(--qm-space-2);
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .lines dt {
    color: var(--qm-text-muted);
  }

  .lines dd {
    margin: 0;
    overflow: hidden;
    white-space: pre;
    text-overflow: ellipsis;
  }

  .lines code {
    font-family: var(--qm-font-mono);
  }

  .warnings {
    margin: 0;
    padding: 0 0 0 var(--qm-space-4);
  }

  .preview {
    height: 240px;
  }
</style>
