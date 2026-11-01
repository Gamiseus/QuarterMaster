<script lang="ts">
  import { Tooltip } from "bits-ui";
  import type { Snippet } from "svelte";

  interface Props {
    text: string;
    side?: "top" | "right" | "bottom" | "left";
    /** Spread `props` onto the focusable element the tooltip describes. */
    children: Snippet<[{ props: Record<string, unknown> }]>;
  }

  let { text, side = "top", children }: Props = $props();
</script>

<!-- Needs one app-level Tooltip.Provider (App.svelte; tests use harness/render.ts). -->
<Tooltip.Root>
  <Tooltip.Trigger>
    {#snippet child({ props })}
      {@render children({ props })}
    {/snippet}
  </Tooltip.Trigger>
  <Tooltip.Portal>
    <Tooltip.Content class="qm-tooltip" role="tooltip" {side} sideOffset={6}>{text}</Tooltip.Content>
  </Tooltip.Portal>
</Tooltip.Root>

<style>
  :global(.qm-tooltip) {
    z-index: var(--qm-z-tooltip);
    max-width: calc(var(--qm-detail-w) - var(--qm-space-4));
    padding: var(--qm-space-1) var(--qm-space-2);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-popover);
    color: var(--qm-text);
    font-size: var(--qm-text-12);
    line-height: 1.4;
    pointer-events: none;
  }
</style>
