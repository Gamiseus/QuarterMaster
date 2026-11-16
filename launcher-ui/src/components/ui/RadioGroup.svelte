<script lang="ts" generics="T">
  import { RadioGroup } from "bits-ui";
  import type { Snippet } from "svelte";

  interface RadioItem {
    value: T;
    label: string;
    description?: string;
    /** The description is for screen readers only, for an item whose `extra` already shows it. */
    descriptionHidden?: boolean;
    disabled?: boolean;
    /** Shown as visible muted text on a disabled item (and its accessible description). */
    disabledReason?: string;
  }

  interface Props {
    value?: T;
    items: RadioItem[];
    label: string;
    class?: string;
    /** Keep the group label for screen readers only (a Section heading already shows it). */
    hideLabel?: boolean;
    /** Extra content under an item's text, such as a preview; decorative (hidden from screen readers). */
    extra?: Snippet<[T]>;
  }

  let { value = $bindable(), items, label, class: className = "", hideLabel = false, extra }: Props = $props();

  const uid = $props.id();
  const labelId = `${uid}-label`;

  // bits-ui values are strings; use the item index so T can be any type.
  const selectedIndex = $derived(items.findIndex((item) => item.value === value));
  const selectedKey = $derived(selectedIndex < 0 ? "" : String(selectedIndex));

  function onValueChange(key: string) {
    const item = items[Number(key)];
    if (item && !item.disabled) value = item.value;
  }

  function describedBy(item: RadioItem, i: number): string | undefined {
    const ids = [
      item.description ? `${uid}-desc-${i}` : "",
      item.disabled && item.disabledReason ? `${uid}-reason-${i}` : "",
    ].filter(Boolean);
    return ids.length ? ids.join(" ") : undefined;
  }
</script>

<div class={["field", className]}>
  <span id={labelId} class={["group-label", { "qm-visually-hidden": hideLabel }]}>{label}</span>
  <RadioGroup.Root class="qm-radio-group" value={selectedKey} {onValueChange} aria-labelledby={labelId}>
    {#each items as item, i (i)}
      <RadioGroup.Item
        class="qm-radio-item"
        value={String(i)}
        disabled={item.disabled ?? false}
        aria-labelledby="{uid}-label-{i}"
        aria-describedby={describedBy(item, i)}
      >
        {#snippet children({ checked })}
          <span class={["dot", { on: checked, dim: item.disabled }]} aria-hidden="true"></span>
          <span class="text">
            <span id="{uid}-label-{i}" class={["item-label", { dim: item.disabled }]}>{item.label}</span>
            {#if item.description}
              <span id="{uid}-desc-{i}" class={["item-desc", { "qm-visually-hidden": item.descriptionHidden }]}>{item.description}</span>
            {/if}
            {#if item.disabled && item.disabledReason}
              <span id="{uid}-reason-{i}" class="item-reason">{item.disabledReason}</span>
            {/if}
            {#if extra}
              <span class="item-extra" aria-hidden="true">{@render extra(item.value)}</span>
            {/if}
          </span>
        {/snippet}
      </RadioGroup.Item>
    {/each}
  </RadioGroup.Root>
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .group-label {
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }

  .field :global(.qm-radio-group) {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  .field :global(.qm-radio-item) {
    display: flex;
    align-items: flex-start;
    gap: var(--qm-space-2);
    padding: var(--qm-space-2);
    border: 1px solid transparent;
    border-radius: var(--qm-radius-sm);
    background: transparent;
    text-align: left;
    transition: border-color var(--qm-dur-fast) var(--qm-ease);
  }

  .field :global(.qm-radio-item:hover:not([data-disabled])) {
    border-color: var(--qm-border);
  }

  .field :global(.qm-radio-item[data-state="checked"]) {
    border-color: var(--qm-accent);
  }

  .dot {
    flex: none;
    width: var(--qm-control-box);
    height: var(--qm-control-box);
    border: 1px solid var(--qm-border-strong);
    border-radius: 50%;
    background: var(--qm-canvas);
  }

  .dot.on {
    border-color: var(--qm-accent);
    background: radial-gradient(circle, var(--qm-accent) 0 40%, var(--qm-canvas) 45%);
  }

  .text {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  .item-label {
    font-size: var(--qm-text-13);
  }

  .item-desc,
  .item-reason {
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }

  /* The reason text stays at full strength so it remains readable. */
  .dim {
    opacity: 0.5;
  }
</style>
