<script lang="ts">
  import { untrack } from "svelte";
  import { Button, VirtualList } from "../../../components/ui";
  import { cssVars } from "../../../lib/actions/cssVars";
  import type { FileDiff } from "../../../lib/bindings";
  import { modColours } from "../../../lib/stores/modcolours.svelte";
  import { alignRows, changeIndices, collapse, viewLines, type ViewLine } from "../../../lib/patches/diff";
  import DiffLine from "./DiffLine.svelte";

  interface Props {
    diff: FileDiff;
    /** Mod display names by id; ids are shown when a name is missing. */
    modNames?: Record<string, string>;
    mode?: "split" | "unified";
    /** Only this mod's changes stay at full strength ("" for every mod): the file list's filter. */
    modFilter?: string;
    /** The picked patch: the view scrolls to its first change when it is picked or the file loads. */
    focusPatch?: string | null;
    /** Unchanged lines kept around each change before the rest fold away. */
    context?: number;
  }

  let { diff, modNames = {}, mode = $bindable("split"), context = 3, focusPatch = null, modFilter = "" }: Props = $props();

  let expanded = $state<ReadonlySet<number>>(new Set());
  /** The line the last Next or Previous went to; -1 before any. */
  let cursor = $state(-1);
  let list: { scrollToIndex: (i: number) => void } | undefined = $state();
  /**
   * Split needs two columns of code. Below this width, in text-size units, each column shows
   * only a few characters (a large text size, a narrow preview), so the view starts unified.
   */
  const SPLIT_MIN_EM = 32;
  let root: HTMLElement | undefined = $state();
  let width = $state(0);
  /** False once the user picked a layout: their pick stays. */
  let auto = $state(true);
  $effect(() => {
    if (!auto || !root || width === 0) return;
    const em = parseFloat(getComputedStyle(root).fontSize) || 13;
    // Two columns that do not fit side by side put the changed side out of view, so a file with
    // lines longer than half the view starts unified too (unless the lines wrap).
    const tooLong = !wrap && longest > splitCols;
    mode = width / em < SPLIT_MIN_EM || tooLong ? "unified" : "split";
  });
  function pick(m: "split" | "unified") {
    auto = false;
    mode = m;
  }

  const WRAP_KEY = "qm.diff.wrap";
  function storedWrap(): boolean {
    try {
      return localStorage.getItem(WRAP_KEY) === "1";
    } catch {
      return false;
    }
  }
  /** Long lines wrap inside their column instead of scrolling sideways. */
  let wrap = $state(storedWrap());
  function toggleWrap() {
    wrap = !wrap;
    try {
      localStorage.setItem(WRAP_KEY, wrap ? "1" : "0");
    } catch {
      // The choice then lasts for this visit only.
    }
  }
  /** A run of 50 code characters, to measure one. */
  let ruler: HTMLElement | undefined = $state();
  let rulerW = $state(0);
  /** How many characters fit one code column. Line numbers, the sign and the scrollbar take about 16 characters. */
  const codeCols = $derived.by(() => {
    void ruler;
    const ch = rulerW > 0 ? rulerW / 50 : 7.3;
    return width / ch - 16;
  });
  /** One column of the split layout, in characters. */
  const splitCols = $derived(Math.max(16, Math.floor(codeCols / 2) - 1));
  const cols = $derived(mode === "split" ? splitCols : Math.max(16, Math.floor(codeCols) - 1));
  /** A line's width in characters; a tab can take up to eight. */
  const span = (text: string | undefined) => {
    if (!text) return 0;
    let n = text.length;
    for (let i = text.indexOf("\t"); i >= 0; i = text.indexOf("\t", i + 1)) n += 7;
    return n;
  };
  const rowSpan = (l: ViewLine) =>
    l.fold ? 1 : Math.max(1, Math.ceil(span(l.left?.text) / cols), Math.ceil(span(l.right?.text) / cols));

  const rows = $derived(
    alignRows(diff.original_lines, diff.result.attributed_lines, diff.result.records, diff.result.patched_line_count),
  );
  const lines = $derived(viewLines(collapse(rows, context, expanded), mode));
  // Over the file's mods, like the patch strip, so a mod has one colour on the whole tab.
  const colours = $derived(modColours([...diff.result.mods_touching, ...rows.flatMap((r) => (r.mod ? [r.mod] : []))]));
  const name = (mod?: string) => (mod ? (modNames[mod] ?? mod) : "");

  const matches = (l: ViewLine) => !modFilter || l.mod === modFilter;
  const filtering = $derived(modFilter !== "");
  /** The longest line, in characters: lines longer than the view scroll sideways instead of being cut. */
  const longest = $derived(
    Math.min(400, rows.reduce((n, r) => Math.max(n, r.left?.text.length ?? 0, r.right?.text.length ?? 0), 0)),
  );
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

  /** Scrolls to the first change of `patchId` in this file, if it has one. */
  export function showPatch(patchId: string): boolean {
    const i = lines.findIndex((l) => l.kind !== "same" && l.kind !== "fold" && l.patch === patchId);
    if (i < 0) return false;
    cursor = i;
    list?.scrollToIndex(i);
    return true;
  }

  // A patch picked elsewhere (a deep link, the order view) is scrolled to once the file shows.
  $effect(() => {
    const id = focusPatch;
    void diff;
    if (!id || !list) return;
    untrack(() => showPatch(id));
  });

  // A different file starts folded and at the top.
  $effect(() => {
    void diff;
    expanded = new Set();
    cursor = -1;
  });
</script>

<div
  class="diff-view"
  bind:this={root}
  bind:clientWidth={width}
  use:cssVars={{ "--diff-len": wrap ? "0" : String(longest), "--diff-cols": mode === "split" ? "2" : "1" }}
>
  <span class="ruler" aria-hidden="true" bind:this={ruler} bind:clientWidth={rulerW}>{"0".repeat(50)}</span>
  <div class="toolbar">
    <div class="modes" role="group" aria-label="Layout">
      <Button size="sm" variant={mode === "split" ? "secondary" : "ghost"} aria-pressed={mode === "split"} onclick={() => pick("split")}>Split</Button>
      <Button size="sm" variant={mode === "unified" ? "secondary" : "ghost"} aria-pressed={mode === "unified"} onclick={() => pick("unified")}>Unified</Button>
      <Button size="sm" variant={wrap ? "secondary" : "ghost"} aria-pressed={wrap} onclick={toggleWrap} title="Wrap long lines instead of scrolling sideways">Wrap</Button>
    </div>
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
    rowSpan={wrap ? rowSpan : undefined}
  >
    {#snippet row(l, i)}
      <DiffLine
        line={l}
        {mode}
        colour={l.mod ? (colours.get(l.mod) ?? 0) : 0}
        tag={tagFor(i)}
        {wrap}
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

  .ruler {
    position: absolute;
    visibility: hidden;
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    white-space: pre;
  }

  /* Wide enough for the longest line in each code column (a monospace character is about 0.61em). */
  .diff-view :global(.lines) {
    --vl-min-w: calc(var(--diff-len, 0) * var(--diff-cols, 1) * var(--qm-text-12) * 0.61 + 30ch);
    flex: 1;
    --vl-row-h: var(--qm-diff-row-h);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-panel);
  }
</style>
