<script lang="ts">
  import { Button, Select, VirtualList } from "../../../components/ui";
  import type { FileDiff } from "../../../lib/bindings";
  import { assignModColours } from "../../../lib/patches/colors";
  import { alignRows, changeIndices, collapse, viewLines, type ViewLine } from "../../../lib/patches/diff";
  import DiffLine from "./DiffLine.svelte";

  interface Props {
    diff: FileDiff;
    /** Mod display names by id; ids are shown when a name is missing. */
    modNames?: Record<string, string>;
    mode?: "split" | "unified";
    /** Unchanged lines kept around each change before the rest fold away. */
    context?: number;
  }

  let { diff, modNames = {}, mode = $bindable("split"), context = 3 }: Props = $props();

  let expanded = $state<ReadonlySet<number>>(new Set());
  let modFilter = $state("");
  let patchFilter = $state("");
  /** The line the last Next or Previous went to; -1 before any. */
  let cursor = $state(-1);
  let list: { scrollToIndex: (i: number) => void } | undefined = $state();

  const rows = $derived(
    alignRows(diff.original_lines, diff.result.attributed_lines, diff.result.records, diff.result.patched_line_count),
  );
  const lines = $derived(viewLines(collapse(rows, context, expanded), mode));
  // Over the file's mods, like the patch strip, so a mod has one colour on the whole tab.
  const colours = $derived(
    assignModColours([...diff.result.mods_touching, ...rows.flatMap((r) => (r.mod ? [r.mod] : []))]),
  );
  const name = (mod?: string) => (mod ? (modNames[mod] ?? mod) : "");

  const patchNames = $derived.by(() => {
    const names = new Map<string, string>();
    for (const a of diff.result.attributed_lines) if (a.patch_id) names.set(a.patch_id, a.patch_name ?? a.patch_id);
    for (const r of diff.result.records) names.set(r.patch_id, r.patch_name || r.patch_id);
    return names;
  });
  const modItems = $derived([
    { value: "", label: "All mods" },
    ...[...colours.keys()].map((m) => ({ value: m, label: name(m) })),
  ]);
  const patchItems = $derived([
    { value: "", label: "All patches" },
    ...[...new Set(rows.flatMap((r) => (r.patch && (!modFilter || r.mod === modFilter) ? [r.patch] : [])))].map(
      (p) => ({ value: p, label: patchNames.get(p) ?? p }),
    ),
  ]);

  const matches = (l: ViewLine) => (!modFilter || l.mod === modFilter) && (!patchFilter || l.patch === patchFilter);
  const filtering = $derived(modFilter !== "" || patchFilter !== "");
  /** Starts of the changes the filters keep. */
  const changes = $derived(changeIndices(lines).filter((i) => hunk(i).some(matches)));
  const position = $derived(changes.indexOf(cursor));

  function hunk(start: number): ViewLine[] {
    let end = start;
    while (end < lines.length && lines[end].kind !== "same" && lines[end].kind !== "fold") end++;
    return lines.slice(start, end);
  }

  /** The mod tag shows on the first line of each change and wherever the mod changes. */
  function tagFor(i: number): string {
    const l = lines[i];
    if (l.kind === "same" || l.kind === "fold" || !l.mod) return "";
    const prev = lines[i - 1];
    return prev && prev.kind !== "same" && prev.kind !== "fold" && prev.mod === l.mod ? "" : name(l.mod);
  }

  function go(step: 1 | -1) {
    if (changes.length === 0) return;
    const next =
      step === 1
        ? (changes.find((i) => i > cursor) ?? changes[0])
        : ([...changes].reverse().find((i) => i < cursor) ?? changes[changes.length - 1]);
    cursor = next;
    list?.scrollToIndex(next);
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    if (e.key === "n" || e.key === "N") {
      e.preventDefault();
      go(1);
    } else if (e.key === "p" || e.key === "P") {
      e.preventDefault();
      go(-1);
    }
  }

  function expand(start: number) {
    expanded = new Set([...expanded, start]);
  }

  // A patch filter from another mod would match nothing.
  $effect(() => {
    if (modFilter && patchFilter && !patchItems.some((p) => p.value === patchFilter)) patchFilter = "";
  });

  // A different file starts folded and at the top.
  $effect(() => {
    void diff;
    expanded = new Set();
    cursor = -1;
    modFilter = "";
    patchFilter = "";
  });
</script>

<div class="diff-view">
  <div class="toolbar">
    <div class="modes" role="group" aria-label="Layout">
      <Button size="sm" variant={mode === "split" ? "secondary" : "ghost"} aria-pressed={mode === "split"} onclick={() => (mode = "split")}>Split</Button>
      <Button size="sm" variant={mode === "unified" ? "secondary" : "ghost"} aria-pressed={mode === "unified"} onclick={() => (mode = "unified")}>Unified</Button>
    </div>
    <Select label="Mod" hideLabel items={modItems} bind:value={modFilter} />
    <Select label="Patch" hideLabel items={patchItems} bind:value={patchFilter} />
    <span class="qm-muted status" role="status">
      {#if changes.length === 0}
        No changes{filtering ? " match" : ""}.
      {:else if position >= 0}
        Change {position + 1} of {changes.length}
      {:else}
        {changes.length} {changes.length === 1 ? "change" : "changes"}
      {/if}
    </span>
    <div class="nav">
      <Button size="sm" variant="ghost" disabled={changes.length === 0} onclick={() => go(-1)} title="Previous change (P)">Previous change</Button>
      <Button size="sm" variant="ghost" disabled={changes.length === 0} onclick={() => go(1)} title="Next change (N)">Next change</Button>
    </div>
  </div>
  <VirtualList
    bind:this={list}
    class="lines"
    items={lines}
    key={(l) => l.key}
    label={`Changes in ${diff.result.target_file}`}
    {onkeydown}
  >
    {#snippet row(l, i)}
      <DiffLine
        line={l}
        {mode}
        colour={l.mod ? (colours.get(l.mod) ?? 0) : 0}
        tag={tagFor(i)}
        dimmed={filtering && l.kind !== "same" && l.kind !== "fold" && !matches(l)}
        onexpand={() => l.fold && expand(l.fold.start)}
      />
    {/snippet}
  </VirtualList>
</div>

<style>
  .diff-view {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
    gap: var(--qm-space-2);
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .modes,
  .nav {
    display: flex;
    gap: var(--qm-space-1);
  }

  .nav {
    margin-left: auto;
  }

  .status {
    font-size: var(--qm-text-12);
  }

  .diff-view :global(.lines) {
    flex: 1;
    --vl-row-h: var(--qm-diff-row-h);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-panel);
  }
</style>
