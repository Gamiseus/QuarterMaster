<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  interface Props extends HTMLAttributes<HTMLElement> {
    children?: Snippet;
  }

  let { class: className = "", children, ...rest }: Props = $props();
</script>

<kbd class={["kbd", className]} {...rest}>{@render children?.()}</kbd>

<style>
  .kbd {
    display: inline-block;
    min-width: var(--qm-kbd-min);
    padding: 1px var(--qm-space-2);
    border: 1px solid var(--qm-border);
    border-bottom-width: 2px;
    border-radius: var(--qm-radius-sm);
    background: var(--qm-raised);
    color: var(--qm-text-muted);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-11);
    line-height: 1.4;
    text-align: center;
  }
</style>
