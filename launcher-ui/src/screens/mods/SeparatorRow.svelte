<script lang="ts">
  import type { Attachment } from "svelte/attachments";
  import { IconChevronDown, IconEdit, IconGrip, IconTrash } from "../../components/icons";
  import { IconButton } from "../../components/ui";
  import type { ProfileSeparator } from "../../lib/bindings";
  import { sepColorIndex } from "../../lib/modlist/view";

  interface Props {
    sep: ProfileSeparator;
    /** Mods in this section (shown while collapsed). */
    count: number;
    selected: boolean;
    active: boolean;
    canMutate: boolean;
    handle: Attachment<HTMLElement>;
    ontogglecollapse: () => void;
    onedit: () => void;
    onremove: () => void;
  }

  let { sep, count, selected, active, canMutate, handle, ontogglecollapse, onedit, onremove }: Props = $props();

  const tab = $derived(active ? 0 : -1);
  const collapsed = $derived(sep.collapsed === true);
</script>

<div
  role="row"
  class={["row", `sep-${sepColorIndex(sep.color)}`, { selected }]}
  tabindex={tab}
  aria-selected={selected}
  aria-label={`Separator ${sep.title}`}
  data-key={`sep:${sep.id}`}
>
  <div role="gridcell" class="cell">
    <span class="handle" aria-hidden="true" {@attach handle}><IconGrip /></span>
  </div>
  <div role="gridcell" class="cell title">
    <span class="qm-stencil label">{sep.title}</span>
    {#if collapsed}
      <span class="count">{count} {count === 1 ? "mod" : "mods"} hidden</span>
    {/if}
  </div>
  <div role="gridcell" class="cell actions">
    <IconButton
      icon={IconChevronDown}
      label={collapsed ? "Expand section" : "Collapse section"}
      class={collapsed ? "chevron collapsed" : "chevron"}
      aria-expanded={!collapsed}
      disabled={!canMutate}
      tabindex={tab}
      onclick={ontogglecollapse}
    />
    <IconButton icon={IconEdit} label="Edit separator" disabled={!canMutate} tabindex={tab} onclick={onedit} />
    <IconButton icon={IconTrash} label="Remove separator" disabled={!canMutate} tabindex={tab} onclick={onremove} />
  </div>
</div>

<style>
  .row {
    --sep: var(--qm-sep-1);
    display: grid;
    grid-template-columns: var(--qm-space-5) minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--qm-space-2);
    min-height: var(--qm-row-h);
    padding: 0 var(--qm-space-2);
    border-bottom: 1px solid var(--qm-border);
    background: color-mix(in srgb, var(--sep) 16%, var(--qm-panel));
    box-shadow: inset 4px 0 0 var(--sep);
    user-select: none;
  }

  .sep-2 {
    --sep: var(--qm-sep-2);
  }

  .sep-3 {
    --sep: var(--qm-sep-3);
  }

  .sep-4 {
    --sep: var(--qm-sep-4);
  }

  .sep-5 {
    --sep: var(--qm-sep-5);
  }

  .sep-6 {
    --sep: var(--qm-sep-6);
  }

  .row.selected {
    background: color-mix(in srgb, var(--sep) 30%, var(--qm-panel));
  }

  .row:focus-visible {
    box-shadow:
      inset 4px 0 0 var(--sep),
      var(--qm-focus-ring);
  }

  .cell {
    display: flex;
    align-items: center;
    min-width: 0;
  }

  .handle {
    display: inline-flex;
    color: var(--qm-text-muted);
    cursor: grab;
  }

  .title {
    gap: var(--qm-space-3);
  }

  .label {
    overflow: hidden;
    color: var(--qm-text);
    font-size: var(--qm-text-12);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .count {
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }

  .actions {
    gap: var(--qm-space-1);
  }

  .actions :global(.chevron .qm-icon) {
    transition: transform var(--qm-dur-fast) var(--qm-ease);
  }

  .actions :global(.chevron.collapsed .qm-icon) {
    transform: rotate(-90deg);
  }
</style>
