<script lang="ts">
  import { IconPlus } from "../../components/icons";
  import { Button, TextField } from "../../components/ui";
  import type { FilterChip } from "../../lib/modlist/filter";

  interface Props {
    text: string;
    chips: Set<FilterChip>;
    canMutate: boolean;
    onaddseparator: () => void;
  }

  let { text = $bindable(""), chips = $bindable(new Set()), canMutate, onaddseparator }: Props = $props();

  const CHIPS: { value: FilterChip; label: string }[] = [
    { value: "enabled", label: "Enabled" },
    { value: "disabled", label: "Disabled" },
    { value: "conflicts", label: "Conflicts" },
    { value: "updates", label: "Updates" },
  ];

  function toggle(chip: FilterChip) {
    const next = new Set(chips);
    if (!next.delete(chip)) next.add(chip);
    chips = next;
  }
</script>

<div class="bar">
  <TextField class="search" label="Filter mods" placeholder="Name or id" bind:value={text} />
  <div class="chips" role="group" aria-label="Show only">
    {#each CHIPS as chip (chip.value)}
      <button
        type="button"
        class={["chip", { on: chips.has(chip.value) }]}
        aria-pressed={chips.has(chip.value)}
        onclick={() => toggle(chip.value)}
      >
        {chip.label}
      </button>
    {/each}
  </div>
  <Button class="add" disabled={!canMutate} onclick={onaddseparator}><IconPlus />Add separator</Button>
</div>

<style>
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--qm-space-3);
  }

  .bar :global(.search) {
    flex: 1;
    min-width: calc(var(--qm-detail-w) / 2);
  }

  .chips {
    display: flex;
    gap: var(--qm-space-1);
  }

  .chip {
    min-height: var(--qm-row-h);
    padding: var(--qm-space-1) var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: transparent;
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
    transition:
      background var(--qm-dur-fast) var(--qm-ease),
      border-color var(--qm-dur-fast) var(--qm-ease),
      color var(--qm-dur-fast) var(--qm-ease);
  }

  .chip:hover {
    border-color: var(--qm-accent);
    color: var(--qm-text);
  }

  .chip.on {
    border-color: var(--qm-accent);
    background: color-mix(in srgb, var(--qm-accent) 16%, transparent);
    color: var(--qm-text);
  }
</style>
