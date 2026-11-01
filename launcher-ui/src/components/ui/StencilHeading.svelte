<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  interface Props extends HTMLAttributes<HTMLHeadingElement> {
    level: 1 | 2 | 3;
    children?: Snippet;
  }

  let { level, class: className = "", children, ...rest }: Props = $props();
</script>

<svelte:element this={`h${level}`} class={["heading", `level-${level}`, className]} {...rest}>
  {@render children?.()}
</svelte:element>

<style>
  /* Family, weight, tracking and case come from base.css (h1-h3); only sizes are set here. */
  .level-1 {
    font-size: var(--qm-text-20);
  }

  .level-2 {
    font-size: var(--qm-text-16);
  }

  .level-3 {
    font-size: var(--qm-text-13);
    color: var(--qm-text-muted);
  }
</style>
