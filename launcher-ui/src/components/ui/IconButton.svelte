<script lang="ts">
  import type { Component } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";
  import { mergeProps } from "svelte-toolbelt";
  import Tooltip from "./Tooltip.svelte";

  interface Props extends HTMLButtonAttributes {
    icon: Component;
    /** Accessible name; also the tooltip text. */
    label: string;
  }

  let { icon: Icon, label, disabled = false, type = "button", class: className = "", ...rest }: Props = $props();
</script>

<Tooltip text={label}>
  {#snippet children({ props })}
    <button
      {...mergeProps(props, rest)}
      class={["icon-btn", className]}
      {type}
      {disabled}
      aria-label={label}
    >
      <Icon />
    </button>
  {/snippet}
</Tooltip>

<style>
  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--qm-row-h);
    height: var(--qm-row-h);
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--qm-radius-sm);
    background: transparent;
    color: var(--qm-text-muted);
    transition:
      background var(--qm-dur-fast) var(--qm-ease),
      color var(--qm-dur-fast) var(--qm-ease),
      border-color var(--qm-dur-fast) var(--qm-ease);
  }

  .icon-btn:hover:not(:disabled) {
    background: var(--qm-raised);
    border-color: var(--qm-border);
    color: var(--qm-text);
  }

  .icon-btn:disabled {
    opacity: 0.5;
  }
</style>
