<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    tone: "info" | "warn" | "danger";
    title?: string;
    children: Snippet;
    action?: Snippet;
    class?: string;
  }

  let { tone, title, children, action, class: className = "" }: Props = $props();
</script>

<div class={["callout", `tone-${tone}`, className]} role="note">
  <div class="content">
    {#if title}
      <p class="title">{title}</p>
    {/if}
    <div class="body">{@render children()}</div>
  </div>
  {#if action}
    <div class="action">{@render action()}</div>
  {/if}
</div>

<style>
  /* The fill matches Badge (tone at 14%), so the tone-colored title is covered by the contrast test. */
  .callout {
    --tone: var(--qm-accent-text);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-3);
    padding: var(--qm-space-3) var(--qm-space-4);
    border: 1px solid color-mix(in srgb, var(--tone) 55%, transparent);
    border-radius: var(--qm-radius-md);
    background: color-mix(in srgb, var(--tone) 14%, transparent);
    color: var(--qm-text);
    font-size: var(--qm-text-13);
  }

  .tone-warn {
    --tone: var(--qm-warn);
  }

  .tone-danger {
    --tone: var(--qm-danger);
  }

  .content {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    min-width: 0;
  }

  .title {
    margin: 0;
    color: var(--tone);
    font-weight: 600;
  }

  .body :global(p) {
    margin: 0;
  }

  .action {
    flex: none;
  }
</style>
