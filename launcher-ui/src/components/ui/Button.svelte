<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  interface Props extends HTMLButtonAttributes {
    variant?: "primary" | "secondary" | "ghost" | "danger";
    size?: "sm" | "md";
    children?: Snippet;
  }

  let {
    variant = "secondary",
    size = "md",
    disabled = false,
    type = "button",
    class: className = "",
    children,
    ...rest
  }: Props = $props();
</script>

<button class={["btn", `variant-${variant}`, `size-${size}`, className]} {type} {disabled} {...rest}>
  {@render children?.()}
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--qm-space-2);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-raised);
    color: var(--qm-text);
    font-size: var(--qm-text-13);
    line-height: 1.2;
    white-space: nowrap;
    transition:
      background var(--qm-dur-fast) var(--qm-ease),
      border-color var(--qm-dur-fast) var(--qm-ease),
      color var(--qm-dur-fast) var(--qm-ease);
  }

  .size-md {
    min-height: var(--qm-row-h);
    padding: var(--qm-space-2) var(--qm-space-4);
  }

  .size-sm {
    padding: var(--qm-space-1) var(--qm-space-3);
    font-size: var(--qm-text-12);
  }

  .btn:hover:not(:disabled) {
    border-color: var(--qm-accent);
  }

  .variant-primary {
    border-color: var(--qm-accent);
    background: var(--qm-accent);
    color: var(--qm-accent-contrast);
    font-weight: 600;
  }

  .variant-primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--qm-accent) 85%, var(--qm-text));
  }

  .variant-ghost {
    border-color: transparent;
    background: transparent;
  }

  .variant-ghost:hover:not(:disabled) {
    background: var(--qm-raised);
    border-color: var(--qm-border);
  }

  .variant-danger {
    border-color: var(--qm-danger);
    color: var(--qm-danger);
  }

  .variant-danger:hover:not(:disabled) {
    background: var(--qm-danger);
    border-color: var(--qm-danger);
    color: var(--qm-accent-contrast);
  }

  .btn:disabled {
    opacity: 0.5;
  }
</style>
