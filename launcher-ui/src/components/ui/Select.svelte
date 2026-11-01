<script lang="ts" generics="T">
  import { Select } from "bits-ui";
  import IconCheck from "../icons/IconCheck.svelte";
  import IconChevronDown from "../icons/IconChevronDown.svelte";
  import type { SelectTriggerProps } from "bits-ui";

  // `class` styles the wrapper; any other attribute (data-*, aria-*, id, ...) goes to the trigger button.
  interface Props extends Omit<SelectTriggerProps, "children" | "child" | "ref" | "value"> {
    value?: T;
    items: { value: T; label: string }[];
    label: string;
    placeholder?: string;
  }

  let {
    value = $bindable(),
    items,
    label,
    placeholder = "Select...",
    disabled = false,
    class: className = "",
    ...rest
  }: Props = $props();

  const uid = $props.id();
  const labelId = `${uid}-label`;
  const valueId = `${uid}-value`;

  // bits-ui values are strings; use the item index so T can be any type.
  const selectedIndex = $derived(items.findIndex((item) => item.value === value));
  const selectedKey = $derived(selectedIndex < 0 ? "" : String(selectedIndex));
  const selectedLabel = $derived(selectedIndex < 0 ? placeholder : items[selectedIndex].label);
  const bitsItems = $derived(items.map((item, i) => ({ value: String(i), label: item.label })));

  function onValueChange(key: string) {
    const item = items[Number(key)];
    if (item) value = item.value;
  }
</script>

<div class={["field", className]}>
  <span id={labelId} class="label">{label}</span>
  <Select.Root type="single" value={selectedKey} {onValueChange} items={bitsItems} disabled={disabled ?? false}>
    <Select.Trigger {...rest} class="qm-select-trigger" aria-labelledby="{labelId} {valueId}">
      <span id={valueId} class={{ placeholder: selectedIndex < 0 }}>{selectedLabel}</span>
      <IconChevronDown />
    </Select.Trigger>
    <Select.Portal>
      <Select.Content class="qm-select-content" sideOffset={4}>
        <Select.Viewport>
          {#each bitsItems as item (item.value)}
            <Select.Item class="qm-select-item" value={item.value} label={item.label}>
              {#snippet children({ selected })}
                <span>{item.label}</span>
                {#if selected}
                  <IconCheck />
                {/if}
              {/snippet}
            </Select.Item>
          {/each}
        </Select.Viewport>
      </Select.Content>
    </Select.Portal>
  </Select.Root>
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  .label {
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }

  .placeholder {
    color: var(--qm-text-muted);
  }

  .field :global(.qm-select-trigger) {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-2);
    min-height: var(--qm-row-h);
    padding: var(--qm-space-1) var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
    color: var(--qm-text);
    font-size: var(--qm-text-13);
    text-align: left;
    transition: border-color var(--qm-dur-fast) var(--qm-ease);
  }

  .field :global(.qm-select-trigger:hover:not([data-disabled])) {
    border-color: var(--qm-accent);
  }

  .field :global(.qm-select-trigger[data-disabled]) {
    opacity: 0.5;
  }

  /* The list is portalled out of .field, so these rules cannot be scoped to it. */
  :global(.qm-select-content) {
    z-index: var(--qm-z-popover);
    min-width: var(--bits-select-anchor-width);
    max-height: calc(var(--qm-row-h) * 8);
    overflow-y: auto;
    padding: var(--qm-space-1);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-md);
    background: var(--qm-popover);
    color: var(--qm-text);
    font-size: var(--qm-text-13);
  }

  :global(.qm-select-item) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-3);
    padding: var(--qm-space-2) var(--qm-space-3);
    border-radius: var(--qm-radius-sm);
    cursor: pointer;
  }

  :global(.qm-select-item[data-highlighted]) {
    background: var(--qm-raised);
  }

  :global(.qm-select-item[data-selected]) {
    color: var(--qm-accent);
  }

  :global(.qm-select-item[data-disabled]) {
    opacity: 0.5;
    cursor: default;
  }
</style>
