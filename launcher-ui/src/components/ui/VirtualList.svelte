<script lang="ts" generics="T">
  import type { Snippet } from "svelte";
  import { cssVars } from "../../lib/actions/cssVars";

  interface Props {
    items: T[];
    row: Snippet<[T, number]>;
    /** A stable key per item. */
    key: (item: T, index: number) => string | number;
    /** Names the scroll region (it is focusable so keyboard users can scroll it). */
    label: string;
    /** Rows rendered above and below the visible ones. */
    overscan?: number;
    /** Used until the real sizes are measured (and in environments that cannot measure). */
    fallbackRowHeight?: number;
    fallbackViewport?: number;
    class?: string;
    onkeydown?: (e: KeyboardEvent) => void;
    /** How many rows tall an item is (wrapped text); every item is one row without it. */
    rowSpan?: (item: T, index: number) => number;
  }

  let {
    items,
    row,
    key,
    label,
    overscan = 12,
    fallbackRowHeight = 20,
    fallbackViewport = 600,
    class: className = "",
    onkeydown,
    rowSpan,
  }: Props = $props();

  let viewport: HTMLDivElement | undefined = $state();
  let probe: HTMLDivElement | undefined = $state();
  let rowH = $state(0);
  let viewH = $state(0);
  let scrollTop = $state(0);

  // Row height comes from the `--vl-row-h` the caller sets (a token, so it follows the text size).
  const h = $derived(rowH > 0 ? rowH : fallbackRowHeight);
  const vh = $derived(viewH > 0 ? viewH : fallbackViewport);
  /** With `rowSpan`: the row each item starts at, and one more entry for the end. */
  const starts = $derived.by(() => {
    if (!rowSpan) return null;
    const out = new Array<number>(items.length + 1);
    out[0] = 0;
    for (let i = 0; i < items.length; i++) out[i + 1] = out[i] + Math.max(1, Math.round(rowSpan(items[i], i)));
    return out;
  });
  const startOf = (i: number) => (starts ? starts[i] : i);
  /** The item that covers row `r`. */
  function itemAt(r: number): number {
    if (!starts) return Math.floor(r);
    let lo = 0;
    let hi = items.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (starts[mid + 1] <= r) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }
  const first = $derived(Math.max(0, itemAt(scrollTop / h) - overscan));
  const last = $derived(Math.min(items.length, itemAt((scrollTop + vh) / h) + 1 + overscan));
  const shown = $derived(items.slice(first, last));
  const NO_VARS: Record<string, string> = {};

  $effect(() => {
    if (!viewport || !probe) return;
    const measure = () => {
      rowH = probe?.getBoundingClientRect().height ?? 0;
      viewH = viewport?.clientHeight ?? 0;
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(viewport);
    ro.observe(probe);
    return () => ro.disconnect();
  });

  /** Scrolls so row `index` sits in the middle of the view. */
  export function scrollToIndex(index: number) {
    if (!viewport) return;
    const target = Math.max(0, startOf(index) * h - (vh - h) / 2);
    viewport.scrollTop = target;
    // jsdom and some layouts clamp or ignore it; keep the window right either way.
    scrollTop = viewport.scrollTop || target;
  }
</script>

<!-- A focusable scroll region; callers may add keys (the diff view's N and P). -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  bind:this={viewport}
  class={["vl", className]}
  role="region"
  aria-label={label}
  tabindex="0"
  onscroll={() => (scrollTop = viewport?.scrollTop ?? 0)}
  {onkeydown}
>
  <div class="probe" bind:this={probe} aria-hidden="true"></div>
  <div class="spacer" use:cssVars={{ "--vl-total": `${startOf(items.length) * h}px`, "--vl-offset": `${startOf(first) * h}px` }}>
    <div class="window">
      {#each shown as item, k (key(item, first + k))}
        <div
          class="vl-row"
          data-index={first + k}
          use:cssVars={starts ? { "--vl-span": String(starts[first + k + 1] - starts[first + k]) } : NO_VARS}
        >
          {@render row(item, first + k)}
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .vl {
    position: relative;
    min-height: 0;
    overflow: auto;
    scrollbar-gutter: stable;
  }

  .vl:focus-visible {
    outline: none;
    box-shadow: inset var(--qm-focus-ring);
  }

  .probe {
    position: absolute;
    visibility: hidden;
    height: var(--vl-row-h, var(--qm-row-h));
  }

  /* `--vl-min-w` (set by the caller) makes the rows wider than the view, so it scrolls sideways. */
  .spacer {
    position: relative;
    min-width: var(--vl-min-w, 0);
    height: var(--vl-total);
  }

  .window {
    position: absolute;
    top: var(--vl-offset);
    left: 0;
    right: 0;
  }

  .vl-row {
    height: calc(var(--vl-row-h, var(--qm-row-h)) * var(--vl-span, 1));
    overflow: hidden;
  }
</style>
