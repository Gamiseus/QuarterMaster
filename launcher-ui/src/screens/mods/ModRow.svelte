<script lang="ts">
  import type { Attachment } from "svelte/attachments";
  import { IconGrip, IconLock, IconUnlock } from "../../components/icons";
  import { Badge, Checkbox, IconButton, Tooltip } from "../../components/ui";
  import type { ModSummary } from "../../lib/bindings";
  import type { ModStatus } from "../../lib/modlist/status";

  interface Props {
    mod: ModSummary;
    enabled: boolean;
    status: ModStatus | undefined;
    /** 1-based resolved load position; null when the mod is not in the resolved order. */
    position: number | null;
    /** Solver reasons when the solver moved this mod; null when it kept the user's order. */
    movedReasons: string[] | null;
    locked: boolean;
    selected: boolean;
    /** The list's roving tab stop. */
    active: boolean;
    canMutate: boolean;
    handle: Attachment<HTMLElement>;
    onenabledchange: (enabled: boolean) => void;
    ontogglelock: () => void;
  }

  let {
    mod,
    enabled,
    status,
    position,
    movedReasons,
    locked,
    selected,
    active,
    canMutate,
    handle,
    onenabledchange,
    ontogglelock,
  }: Props = $props();

  const tab = $derived(active ? 0 : -1);
  const movedText = $derived(
    movedReasons && movedReasons.length > 0 ? movedReasons.join("; ") : "Moved by the load order solver.",
  );
  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
</script>

<div
  role="row"
  class={["row", { selected, dimmed: !enabled, locked }]}
  tabindex={tab}
  aria-selected={selected}
  aria-label={mod.name}
  data-key={mod.id}
>
  <div role="gridcell" class="cell handle-cell">
    {#if locked}
      <span class="handle off" aria-hidden="true"><IconGrip /></span>
    {:else}
      <span class="handle" aria-hidden="true" {@attach handle}><IconGrip /></span>
    {/if}
  </div>
  <div role="gridcell" class="cell">
    <Checkbox
      checked={enabled}
      label={`Enable ${mod.name}`}
      hideLabel
      disabled={!canMutate}
      tabindex={tab}
      onCheckedChange={(v: boolean) => onenabledchange(v)}
    />
  </div>
  <div role="gridcell" class="cell pos">{enabled && position !== null ? position : ""}</div>
  <div role="gridcell" class="cell name">
    <span class="title">{mod.name}</span>
    {#if mod.category}
      <Badge>{mod.category}</Badge>
    {/if}
    {#if movedReasons}
      <Tooltip text={movedText}>
        {#snippet children({ props })}
          <button {...props} type="button" class="moved" tabindex={tab} aria-label={`Moved: ${movedText}`}>
            <Badge tone="warn">Moved</Badge>
          </button>
        {/snippet}
      </Tooltip>
    {/if}
  </div>
  <div role="gridcell" class="cell version">{mod.version}</div>
  <div role="gridcell" class="cell status">
    {#if status}
      {#if !enabled}
        <!-- Nothing of a disabled mod is applied: no health claim either way. -->
        <Badge>{plural(status.patches, "patch", "patches")}</Badge>
      {:else}
        {#if status.broken > 0}
          <Badge tone="danger">{status.broken} broken</Badge>
        {/if}
        {#if status.conflicts > 0}
          <Badge tone="warn">{plural(status.conflicts, "conflict", "conflicts")}</Badge>
        {/if}
        {#if status.broken === 0 && status.conflicts === 0}
          <Badge tone="ok">{plural(status.patches, "patch", "patches")}</Badge>
        {/if}
      {/if}
      {#if status.updateAvailable}
        <Badge tone="accent">Update</Badge>
      {/if}
    {/if}
  </div>
  <div role="gridcell" class="cell lock">
    <IconButton
      icon={locked ? IconLock : IconUnlock}
      label={locked ? "Unlock position" : "Lock position"}
      class={locked ? "is-locked" : ""}
      disabled={!canMutate}
      tabindex={tab}
      onclick={ontogglelock}
    />
  </div>
</div>

<style>
  .row {
    display: grid;
    grid-template-columns:
      var(--qm-space-5) var(--qm-space-5) var(--qm-space-6) minmax(0, 1fr)
      calc(var(--qm-space-6) * 2) calc(var(--qm-space-6) * 4) var(--qm-row-h);
    align-items: center;
    gap: var(--qm-space-2);
    min-height: var(--qm-row-h);
    padding: 0 var(--qm-space-2);
    border-bottom: 1px solid var(--qm-border);
    background: var(--qm-panel);
    font-size: var(--qm-text-13);
    user-select: none;
    transition: background var(--qm-dur-fast) var(--qm-ease);
  }

  .row:hover {
    background: var(--qm-raised);
  }

  .row.selected {
    background: color-mix(in srgb, var(--qm-accent) 14%, var(--qm-panel));
    box-shadow: inset 3px 0 0 var(--qm-accent);
  }

  .row:focus-visible {
    box-shadow: var(--qm-focus-ring);
  }

  .row.selected:focus-visible {
    box-shadow:
      inset 3px 0 0 var(--qm-accent),
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

  .handle.off {
    opacity: 0.3;
    cursor: default;
  }

  .pos,
  .version {
    color: var(--qm-text-muted);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  .name {
    gap: var(--qm-space-2);
  }

  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dimmed .title,
  .dimmed .version {
    color: var(--qm-text-muted);
  }

  .moved {
    display: inline-flex;
    padding: 0;
    border: 0;
    border-radius: var(--qm-radius-sm);
    background: transparent;
  }

  .status {
    flex-wrap: wrap;
    gap: var(--qm-space-1);
  }

  .lock {
    justify-content: flex-end;
  }

  .lock :global(.is-locked) {
    color: var(--qm-accent);
  }
</style>
