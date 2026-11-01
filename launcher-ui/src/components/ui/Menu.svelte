<script lang="ts">
  import { DropdownMenu } from "bits-ui";
  import type { Snippet } from "svelte";
  import IconCheck from "../icons/IconCheck.svelte";

  interface MenuItem {
    label: string;
    onselect: () => void;
    danger?: boolean;
    disabled?: boolean;
    /** Shown as muted secondary text on a disabled item (also its aria-description). */
    disabledReason?: string;
    /** Secondary muted line under the label. */
    description?: string;
    /** When defined the item is a checkable menu item and shows a check mark while true. */
    checked?: boolean;
    /** Draws a divider above this item. */
    separatorBefore?: boolean;
  }

  interface Props {
    /** Spread `props` onto the button that opens the menu. */
    trigger: Snippet<[{ props: Record<string, unknown> }]>;
    items: MenuItem[];
    align?: "start" | "center" | "end";
  }

  let { trigger, items, align = "start" }: Props = $props();
</script>

{#snippet itemBody(item: MenuItem)}
  <span class="qm-menu-row">
    <span class="qm-menu-label">{item.label}</span>
    {#if item.checked}
      <span class="qm-menu-check" aria-hidden="true"><IconCheck /></span>
    {/if}
  </span>
  {#if item.description}
    <span class="qm-menu-desc">{item.description}</span>
  {/if}
  {#if item.disabled && item.disabledReason}
    <span class="qm-menu-reason" aria-hidden="true">{item.disabledReason}</span>
  {/if}
{/snippet}

<DropdownMenu.Root>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      {@render trigger({ props })}
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Portal>
    <DropdownMenu.Content class="qm-menu-content" sideOffset={4} {align}>
      {#each items as item, i (i)}
        {#if item.separatorBefore}
          <DropdownMenu.Separator class="qm-menu-separator" />
        {/if}
        {#if item.checked !== undefined}
          <DropdownMenu.CheckboxItem
            class={["qm-menu-item", { danger: item.danger }]}
            checked={item.checked}
            disabled={item.disabled}
            onSelect={item.onselect}
            {...{ "aria-description": item.disabled ? item.disabledReason : undefined }}
          >
            {@render itemBody(item)}
          </DropdownMenu.CheckboxItem>
        {:else}
          <DropdownMenu.Item
            class={["qm-menu-item", { danger: item.danger }]}
            disabled={item.disabled}
            onSelect={item.onselect}
            {...{ "aria-description": item.disabled ? item.disabledReason : undefined }}
          >
            {@render itemBody(item)}
          </DropdownMenu.Item>
        {/if}
      {/each}
    </DropdownMenu.Content>
  </DropdownMenu.Portal>
</DropdownMenu.Root>

<style>
  :global(.qm-menu-content) {
    z-index: var(--qm-z-popover);
    min-width: calc(var(--qm-row-h) * 4);
    padding: var(--qm-space-1);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-md);
    background: var(--qm-popover);
    color: var(--qm-text);
    font-size: var(--qm-text-13);
  }

  :global(.qm-menu-content:focus-visible) {
    box-shadow: none;
  }

  :global(.qm-menu-item) {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--qm-space-1);
    padding: var(--qm-space-2) var(--qm-space-3);
    border-radius: var(--qm-radius-sm);
    cursor: pointer;
    user-select: none;
  }

  :global(.qm-menu-item[data-highlighted]) {
    background: var(--qm-raised);
  }

  :global(.qm-menu-item.danger) {
    color: var(--qm-danger);
  }

  :global(.qm-menu-row) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-3);
    align-self: stretch;
  }

  :global(.qm-menu-check) {
    display: inline-flex;
    color: var(--qm-accent);
  }

  :global(.qm-menu-check .qm-icon) {
    width: var(--qm-text-13);
    height: var(--qm-text-13);
  }

  :global(.qm-menu-separator) {
    height: 1px;
    margin: var(--qm-space-1) 0;
    background: var(--qm-border);
  }

  :global(.qm-menu-reason),
  :global(.qm-menu-desc) {
    color: var(--qm-text-muted);
    font-size: var(--qm-text-11);
  }

  :global(.qm-menu-item[data-disabled]) {
    opacity: 0.5;
    cursor: default;
  }
</style>
