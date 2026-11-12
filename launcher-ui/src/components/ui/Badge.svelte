<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import StatusIcon from "../StatusIcon.svelte";

  interface Props extends HTMLAttributes<HTMLSpanElement> {
    tone?: "ok" | "warn" | "danger" | "info" | "neutral" | "accent";
    /** Status tones show their shape before the text; false hides it (tight spots only). */
    icon?: boolean;
    children?: Snippet;
  }

  let { tone = "neutral", icon = true, class: className = "", children, ...rest }: Props = $props();

  const status = $derived(tone === "ok" || tone === "warn" || tone === "danger" || tone === "info" ? tone : null);
</script>

<span class={["badge", `tone-${tone}`, className]} {...rest}
  >{#if status && icon}<StatusIcon tone={status} class="badge-icon" />{/if}{@render children?.()}</span
>

<style>
  .badge {
    --tone: var(--qm-text-muted);
    display: inline-flex;
    align-items: center;
    gap: 3px;
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

  .tone-info {
    --tone: var(--qm-info);
  }

  .tone-accent {
    --tone: var(--qm-accent-text);
  }

  .badge :global(.badge-icon) {
    flex: none;
    width: 1.05em;
    height: 1.05em;
    stroke-width: 2;
  }
</style>
