<script lang="ts">
  import { Badge, Button, RadioGroup } from "../../../components/ui";
  import type { AnchorCandidate, ASite, Edge, FileDiff, InsertPosition, PreviewResult, RegionSite, SiteKey } from "../../../lib/bindings";
  import type { AnchorPick } from "../../../lib/patches/fixes";
  import { couldNotCheck } from "../../../lib/patches/fixlist";
  import { replacedFixes } from "../../../lib/patches/impact";
  import { fixes } from "../../../lib/stores/fixes.svelte";
  import DiffView from "./DiffView.svelte";

  interface Props {
    /** Lines around the lost line (file mode). */
    candidates: AnchorCandidate[];
    /** Empty outside a conflict (a fix picked again from Fixes): no "Resolves" badge. */
    conflictId: string;
    original: string[];
    modNames: Record<string, string>;
    canMutate: boolean;
    /** Region mode (§11.3): the places of another patch's change, to pick a line inside it. */
    region?: { patchId: string; modName: string; patchName: string; sites: RegionSite[] } | null;
    /** The patch's own places; with more than one, the user picks which one moves (D25). */
    aSites?: ASite[];
    /** The place to start with, by its line in the game's file. */
    aSiteLine?: number | null;
    /** Runs a preview with the patch placed at `pick`. */
    onpreview: (pick: AnchorPick) => Promise<PreviewResult | null>;
    onapply: (pick: AnchorPick) => void;
  }

  let { candidates, conflictId, original, modNames, canMutate, region = null, aSites = [], aSiteLine = null, onpreview, onapply }: Props = $props();

  /** What is picked, before the position and place are added. */
  type Picked =
    | { kind: "line"; key: string; text: string }
    | { kind: "region"; key: string; site: number; text: string; nth: number | null }
    | { kind: "edge"; key: string; site: number; edge: Edge; boundary: string };

  let picked = $state<Picked | null>(null);
  let position = $state<InsertPosition>("after");
  // svelte-ignore state_referenced_locally
  let siteIndex = $state(Math.max(0, aSites.findIndex((s) => s.anchor_line === aSiteLine)));
  let preview = $state<PreviewResult | null>(null);
  let checking = $state(false);
  /** Why the last preview failed (another preview replaced it, the simulation is running...). */
  let failed = $state<string | null>(null);
  let seq = 0;

  const aSite = $derived<SiteKey | null>(aSites.length > 1 ? (aSites[siteIndex]?.key ?? null) : null);
  const siteItems = $derived(aSites.map((s, i) => ({ value: i, label: `Line ${s.line}: ${s.text.trim()}` })));

  function pickOf(p: Picked): AnchorPick {
    switch (p.kind) {
      case "line":
        return { kind: "line", text: p.text, position, aSite };
      case "region":
        return { kind: "region", site: p.site, text: p.text, nth: p.nth, position, aSite };
      case "edge":
        return { kind: "edge", site: p.site, edge: p.edge, boundary: p.boundary, aSite };
    }
  }

  async function refresh() {
    if (!picked) return;
    const mine = ++seq;
    checking = true;
    failed = null;
    let r: PreviewResult | null = null;
    let why: string | null = null;
    try {
      r = await onpreview(pickOf(picked));
    } catch (e) {
      why = couldNotCheck(e);
    }
    if (mine !== seq) return;
    preview = r;
    failed = why;
    checking = false;
  }

  function pick(p: Picked) {
    picked = p;
    void refresh();
  }

  function setPosition(p: InsertPosition) {
    position = p;
    void refresh();
  }

  function setSite(i: number | undefined) {
    siteIndex = i ?? 0;
    void refresh();
  }

  /** Lines of B's place that read the same as line `i`, up to it: the rung's `nth` when there are several. */
  function nthOf(s: RegionSite, i: number): number | null {
    const c = s.lines[i];
    if (!c || (c.matches_in_region ?? 0) <= 1) return null;
    const text = (c.raw_text ?? c.text).trim();
    return s.lines.slice(0, i + 1).filter((l) => l.matches_in_region != null && (l.raw_text ?? l.text).trim() === text).length;
  }

  /** Who wrote a line of B's place, in words. */
  function tagOf(c: AnchorCandidate): string {
    if (!region) return "";
    const o = c.origin;
    if (c.matches_in_region == null) return o ? `written by ${modNames[o.mod_id] ?? o.mod_id}` : "the game's line";
    return o?.kind === "kept" ? `${region.modName} kept this line` : `written by ${region.modName}`;
  }

  /** B's first or last own line, for an edge rung's check. */
  function boundaryOf(s: RegionSite, edge: Edge): string {
    const own = s.lines.filter((l) => l.matches_in_region != null);
    const l = edge === "start" ? own[0] : own[own.length - 1];
    return l ? (l.raw_text ?? l.text) : "";
  }

  const diff = $derived<FileDiff | null>(preview?.file ? { original_lines: original, result: preview.file } : null);
  const resolves = $derived(preview?.resolved.includes(conflictId) ?? false);
  /** Anchoring here would stop a file from compiling: no Apply. */
  const breaks = $derived(preview?.impact.breaks_file ?? false);
  const uid = $props.id();
</script>

{#snippet edgeRow(s: RegionSite, edge: Edge)}
  {@const key = `edge:${s.site}:${edge}`}
  {@const check = edge === "start" ? s.edge_start : s.edge_end}
  <li>
    <button
      type="button"
      class={["line", "edge", { picked: picked?.key === key }]}
      aria-pressed={picked?.key === key}
      disabled={!check.allowed}
      onclick={() => pick({ kind: "edge", key, site: s.site, edge, boundary: boundaryOf(s, edge) })}
    >
      <span class="n"></span>
      <span class="text">{edge === "start" ? "Start" : "End"} of {region?.modName}'s change</span>
      {#if !check.allowed && check.reason}<span class="qm-muted why">{check.reason}</span>{/if}
    </button>
  </li>
{/snippet}

<div class="picker">
  {#if aSites.length > 1}
    <RadioGroup label="Which place to move" items={siteItems} bind:value={() => siteIndex, setSite} />
  {/if}
  {#if picked?.kind !== "edge"}
    <div class="positions" role="group" aria-label="Where the code goes">
      <Button size="sm" variant={position === "before" ? "secondary" : "ghost"} aria-pressed={position === "before"} onclick={() => setPosition("before")}>Before the line</Button>
      <Button size="sm" variant={position === "after" ? "secondary" : "ghost"} aria-pressed={position === "after"} onclick={() => setPosition("after")}>After the line</Button>
    </div>
  {/if}
  {#if region}
    {#each region.sites as s (s.site)}
      {@const head = `${uid}-site-${s.site}`}
      <p class="group" id={head}>
        {region.modName} › {region.patchName}, {s.first_line === s.last_line ? `line ${s.first_line}` : `lines ${s.first_line}-${s.last_line}`}{s.function ? ` in ${s.function}` : ""}
      </p>
      <ul class="lines" aria-label={`Lines in ${region.modName}'s change`} aria-describedby={head}>
        {@render edgeRow(s, "start")}
        {#each s.lines as c, i (c.line)}
          {@const key = `region:${s.site}:${c.line}`}
          <li>
            <button type="button" class={["line", { picked: picked?.key === key }]} aria-pressed={picked?.key === key} disabled={c.matches_in_region == null || (c.raw_text ?? c.text).trim() === ""}
              onclick={() => pick({ kind: "region", key, site: s.site, text: c.raw_text ?? c.text, nth: nthOf(s, i) })}>
              <span class="n">{c.line}</span>
              <span class="text">{c.text}</span>
              <span class="qm-muted why">{tagOf(c)}{(c.matches_in_region ?? 0) > 1 ? `, ${c.matches_in_region} lines here read the same` : ""}</span>
            </button>
          </li>
        {/each}
        {@render edgeRow(s, "end")}
      </ul>
    {:else}
      <p class="qm-muted">{region.modName}'s change is no longer in this file.</p>
    {/each}
  {:else}
    <ul class="lines" aria-label="Lines to anchor on">
      {#each candidates as c (c.line)}
        {@const key = `line:${c.line}`}
        <li>
          <button
            type="button"
            class={["line", { picked: picked?.key === key }]}
            aria-pressed={picked?.key === key}
            disabled={c.matches_in_file !== 1}
            onclick={() => pick({ kind: "line", key, text: c.raw_text ?? c.text })}
          >
            <span class="n">{c.line}</span>
            <span class="text">{c.text}</span>
            {#if c.matches_in_file > 1}<span class="qm-muted why">matches {c.matches_in_file} lines</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  {#if picked}
    <div class="result">
      {#if checking}
        <Badge>Checking...</Badge>
      {:else if preview}
        {#if breaks}<Badge tone="danger">Breaks {preview.impact.compile_errors_new.join(", ")}</Badge>{:else if conflictId && resolves}<Badge tone="ok">Resolves this conflict</Badge>{:else if conflictId}<Badge tone="warn">Does not resolve it</Badge>{/if}
        {#if (preview.impact.fallbacks_new ?? []).length > 0}<Badge tone="warn">Uses its own line elsewhere</Badge>{/if}
      {/if}
      <Button size="sm" variant="primary" disabled={!canMutate || checking || !preview || breaks} onclick={() => picked && onapply(pickOf(picked))}>
        Apply
      </Button>
    </div>
    {#if failed && !checking}
      <p class="failed" role="alert">{failed} <Button size="sm" variant="ghost" onclick={() => void refresh()}>Check again</Button></p>
    {/if}
    {#if preview && !checking}
      {#each replacedFixes(preview.impact, (id) => fixes.list.find((f) => f.id === id)?.label ?? null) as label (label)}
        <p class="replaces">Replaces your fix "{label}".</p>
      {/each}
    {/if}
    {#if diff}<div class="preview"><DiffView {diff} {modNames} /></div>{/if}
  {/if}
</div>

<style>
  .picker {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .failed {
    margin: 0;
    color: var(--qm-warn);
    font-size: var(--qm-text-13);
  }

  .positions,
  .result {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .group {
    margin: 0;
    font-size: var(--qm-text-12);
    font-weight: 600;
  }

  .lines {
    margin: 0;
    padding: 0;
    list-style: none;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
  }

  .line {
    display: grid;
    grid-template-columns: 5ch minmax(0, 1fr) auto;
    gap: var(--qm-space-2);
    width: 100%;
    padding: 2px var(--qm-space-2);
    border: 0;
    background: none;
    color: var(--qm-text);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    text-align: left;
    cursor: pointer;
  }

  .line.edge {
    font-family: var(--qm-font-sans);
    font-style: italic;
  }

  .line:hover:not(:disabled),
  .line.picked {
    background: var(--qm-raised);
  }

  .line.picked {
    box-shadow: inset 3px 0 0 var(--qm-accent);
  }

  .line:disabled {
    color: var(--qm-text-muted);
    cursor: not-allowed;
  }

  .line:focus-visible {
    outline: none;
    box-shadow: inset var(--qm-focus-ring);
  }

  .n {
    color: var(--qm-text-muted);
    text-align: right;
  }

  .text {
    overflow: hidden;
    white-space: pre;
    text-overflow: ellipsis;
  }

  .why {
    font-family: var(--qm-font-sans);
    font-size: var(--qm-text-11);
    font-style: normal;
  }

  .preview {
    height: 220px;
  }
  .replaces {
    margin: 0;
    font-size: var(--qm-text-12);
  }
</style>
