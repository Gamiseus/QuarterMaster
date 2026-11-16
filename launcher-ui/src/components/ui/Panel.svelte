<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  interface Props extends HTMLAttributes<HTMLDivElement> {
    corners?: boolean;
    children?: Snippet;
  }

  let { corners = false, class: className = "", children, ...rest }: Props = $props();
</script>

<div class={["panel", { corners, "qm-corners": corners }, className]} {...rest}>
  {@render children?.()}
</div>

<style>
  .panel {
    position: relative;
    padding: var(--qm-space-4);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-md);
    background: var(--qm-panel);
  }
</style>
