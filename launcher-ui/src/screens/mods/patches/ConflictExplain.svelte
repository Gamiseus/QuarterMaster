<script lang="ts">
  import { Button, Inset } from "../../../components/ui";
  import type { DetectedConflict, FileDiff, PatchDetail, PatchExecutionRecord, PreviewResult } from "../../../lib/bindings";
  import { actionOf, anchorOf } from "../../../lib/patches/explain";
  import DiffView from "./DiffView.svelte";
  import LuaCode from "./LuaCode.svelte";
  import SameLineCompare from "./SameLineCompare.svelte";

  interface Props {
    conflict: DetectedConflict;
    sentence: string;
    a?: PatchDetail;
    b?: PatchDetail;
    aDependents: string[];
    bDependents: string[];
    records: PatchExecutionRecord[];
    diff: FileDiff | null;
    modNames: Record<string, string>;
    /** Previews of the file with only A, or only B, applied. */
    only: (keep: "a" | "b") => Promise<PreviewResult | null>;
  }

  let { conflict, sentence, a, b, aDependents, bDependents, records, diff, modNames, only }: Props = $props();

  let region = $state<"both" | "a" | "b">("both");
  let shown = $state<FileDiff | null>(null);
  const mod = (id: string | null) => (id ? (modNames[id] ?? id) : "");

  async function show(which: "both" | "a" | "b") {
    region = which;
    if (which === "both" || !diff) {
      shown = diff;
      return;
    }
    const r = await only(which);
    if (region === which) shown = r?.file ? { original_lines: diff.original_lines, result: r.file } : null;
  }

  $effect(() => {
    void diff;
    void show("both");
  });

  const sides = $derived([
    { key: "a", d: a, id: conflict.patch_a_id, modId: conflict.mod_a_id, deps: aDependents },
    ...(conflict.patch_b_id ? [{ key: "b", d: b, id: conflict.patch_b_id, modId: conflict.mod_b_id, deps: bDependents }] : []),
  ]);
</script>

<section class="explain" aria-label="What happened">
  <p class="sentence">{sentence}</p>
  {#if conflict.same_line && conflict.mod_b_id}
    <SameLineCompare detail={conflict.same_line} modA={mod(conflict.mod_a_id)} modB={mod(conflict.mod_b_id)} />
  {/if}
  <div class="sides">
    {#each sides as s (s.key)}
      {@const anchor = anchorOf(s.d)}
      <div class="side">
        <span class="qm-muted">{mod(s.modId)}</span>
        <strong>{s.d?.name ?? s.id}</strong>
        <span>{actionOf(s.d, records.find((r) => r.patch_id === s.id))}</span>
        {#if anchor}<Inset><LuaCode code={anchor.pattern} label={`Anchor of ${s.d?.name ?? s.id}`} /></Inset>{/if}
        {#if s.deps.length > 0}
          <span class="qm-muted">{s.deps.length} {s.deps.length === 1 ? "patch needs" : "patches need"} this one</span>
        {/if}
      </div>
    {/each}
  </div>
  {#if diff && conflict.patch_b_id}
    <div class="region-tabs" role="group" aria-label="Shared region">
      <Button size="sm" variant={region === "a" ? "secondary" : "ghost"} aria-pressed={region === "a"} onclick={() => show("a")}>{mod(conflict.mod_a_id)} only</Button>
      <Button size="sm" variant={region === "b" ? "secondary" : "ghost"} aria-pressed={region === "b"} onclick={() => show("b")}>{mod(conflict.mod_b_id)} only</Button>
      <Button size="sm" variant={region === "both" ? "secondary" : "ghost"} aria-pressed={region === "both"} onclick={() => show("both")}>Both (current order)</Button>
    </div>
  {/if}
  {#if shown}<div class="region"><DiffView diff={shown} {modNames} /></div>{/if}
</section>

<style>
  .explain {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .sentence {
    margin: 0;
  }

  .sides {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: var(--qm-space-3);
  }

  .side {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    min-width: 0;
    padding: var(--qm-space-2) var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
  }

  .region-tabs {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-1);
  }

  .region {
    height: 220px;
  }
</style>
