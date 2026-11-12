<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  interface Props extends HTMLAttributes<HTMLDivElement> {
    /** Names the region; a labelled region is focusable so keyboard users can scroll it. */
    label?: string;
    children?: Snippet;
    /** The scrolling element, for callers that manage scroll position. */
    ref?: HTMLElement | null;
  }

  let { label, children, class: className = "", ref = $bindable(null), ...rest }: Props = $props();
</script>

<!-- The one scroll container for its column: it fills the space it is given and scrolls inside it. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  bind:this={ref}
  class={["qm-scroll", className]}
  role={label ? "region" : undefined}
  aria-label={label}
  tabindex={label ? 0 : undefined}
  {...rest}
>
  {@render children?.()}
</div>
