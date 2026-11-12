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
  }: Props = $props();

  let viewport: HTMLDivElement | undefined = $state();
  let probe: HTMLDivElement | undefined = $state();
  let rowH = $state(0);
  let viewH = $state(0);
  let scrollTop = $state(0);

  // Row height comes from the `--vl-row-h` the caller sets (a token, so it follows the text size).
  const h = $derived(rowH > 0 ? rowH : fallbackRowHeight);
  const vh = $derived(viewH > 0 ? viewH : fallbackViewport);
  const first = $derived(Math.max(0, Math.floor(scrollTop / h) - overscan));
  const last = $derived(Math.min(items.length, Math.ceil((scrollTop + vh) / h) + overscan));
  const shown = $derived(items.slice(first, last));

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
    const target = Math.max(0, index * h - (vh - h) / 2);
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
  <div class="spacer" use:cssVars={{ "--vl-total": `${items.length * h}px`, "--vl-offset": `${first * h}px` }}>
    <div class="window">
      {#each shown as item, k (key(item, first + k))}
        <div class="vl-row" data-index={first + k}>{@render row(item, first + k)}</div>
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

  .spacer {
    position: relative;
    height: var(--vl-total);
  }

  .window {
    position: absolute;
    top: var(--vl-offset);
    left: 0;
    right: 0;
  }

  .vl-row {
    height: var(--vl-row-h, var(--qm-row-h));
    overflow: hidden;
  }
</style>
