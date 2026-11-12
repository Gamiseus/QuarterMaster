<script lang="ts">
  import { IconMore, IconPlus } from "../../components/icons";
  import { Button, IconButton, Menu, TextField } from "../../components/ui";
  import type { FilterChip } from "../../lib/modlist/filter";

  interface Props {
    text: string;
    chips: Set<FilterChip>;
    canMutate: boolean;
    onaddseparator: () => void;
    onaddmod: () => void;
    /** Mods with an installable update; the Update all button shows when above zero. */
    updateCount?: number;
    /** Why updates cannot be installed right now, or null. */
    updateBlocked?: string | null;
    onupdateall?: () => void;
    /** Narrow: Add separator moves into a More menu; Update all and Add mod stay buttons. */
    compact?: boolean;
  }

  let {
    text = $bindable(""),
    chips = $bindable(new Set()),
    canMutate,
    onaddseparator,
    onaddmod,
    updateCount = 0,
    updateBlocked = null,
    onupdateall,
    compact = false,
  }: Props = $props();

  /** The bar also goes compact when its own column is too narrow for every button (0: not measured yet). */
  let barWidth = $state(0);
  const COMPACT_BELOW = 640;
  const isCompact = $derived(compact || (barWidth > 0 && barWidth < COMPACT_BELOW));

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

  // Update all stays a button (updates are time-sensitive); only Add separator folds into More.
  const moreItems = $derived([
    {
      label: "Add separator",
      disabled: !canMutate,
      disabledReason: canMutate ? undefined : "Available once startup has finished.",
      onselect: onaddseparator,
    },
  ]);
</script>

<div class="bar" bind:clientWidth={barWidth}>
  <div class="top">
    <TextField class="search" label="Filter mods" placeholder="Name or id" bind:value={text} />
    <div class="actions">
      {#if updateCount > 0}
        <Button disabled={updateBlocked !== null} title={updateBlocked ?? undefined} onclick={onupdateall}>
          Update all ({updateCount})
        </Button>
      {/if}
      {#if isCompact}
        <Menu items={moreItems} align="end">
          {#snippet trigger({ props })}
            <IconButton icon={IconMore} label="More actions" {...props} />
          {/snippet}
        </Menu>
      {:else}
        <Button disabled={!canMutate} onclick={onaddseparator}><IconPlus />Add separator</Button>
      {/if}
      <Button variant="primary" disabled={!canMutate} onclick={onaddmod}><IconPlus />Add mod</Button>
    </div>
  </div>
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
</div>

<style>
  .bar {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .top {
    display: flex;
    align-items: flex-end;
    gap: var(--qm-space-3);
  }

  .top :global(.search) {
    flex: 1;
    min-width: 0;
  }

  .chips {
    display: flex;
    gap: var(--qm-space-1);
  }

  .actions {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--qm-space-2);
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
