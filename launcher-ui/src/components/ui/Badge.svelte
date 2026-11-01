<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  interface Props extends HTMLAttributes<HTMLSpanElement> {
    tone?: "ok" | "warn" | "danger" | "neutral" | "accent";
    children?: Snippet;
  }

  let { tone = "neutral", class: className = "", children, ...rest }: Props = $props();
</script>

<span class={["badge", `tone-${tone}`, className]} {...rest}>{@render children?.()}</span>

<style>
  .badge {
    --tone: var(--qm-text-muted);
    display: inline-flex;
    align-items: center;
    padding: 1px var(--qm-space-2);
    border: 1px solid color-mix(in srgb, var(--tone) 55%, transparent);
    border-radius: var(--qm-radius-sm);
    background: color-mix(in srgb, var(--tone) 14%, transparent);
    color: var(--tone);
    font-size: var(--qm-text-11);
    font-weight: 600;
    line-height: 1.5;
    white-space: nowrap;
  }

  .tone-ok {
    --tone: var(--qm-ok);
  }

  .tone-warn {
    --tone: var(--qm-warn);
  }

  .tone-danger {
    --tone: var(--qm-danger);
  }

  .tone-accent {
    --tone: var(--qm-accent);
  }
</style>
