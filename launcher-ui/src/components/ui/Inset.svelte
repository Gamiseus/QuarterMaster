<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  interface Props extends HTMLAttributes<HTMLDivElement> {
    children?: Snippet;
  }

  let { class: className = "", children, ...rest }: Props = $props();
</script>

<!-- A recessed box for nested content (a code snippet, a list item group) inside a card. -->
<div class={["qm-inset", className]} {...rest}>
  {@render children?.()}
</div>

<style>
  .qm-inset {
    padding: var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-raised);
  }
</style>
