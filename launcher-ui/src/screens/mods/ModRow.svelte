<script lang="ts">
  import type { Attachment } from "svelte/attachments";
  import { IconGrip, IconLock, IconUnlock } from "../../components/icons";
  import { Badge, Checkbox, IconButton, Tooltip } from "../../components/ui";
  import type { ModSummary } from "../../lib/bindings";
  import { maxRowBadges, rowBadges, type ModStatus } from "../../lib/modlist/status";
  import { app } from "../../lib/stores/app.svelte";

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
  const badges = $derived(status ? rowBadges(status, enabled, mod.kind, mod.faction) : []);
  const max = $derived(maxRowBadges(app.settings?.text_scale ?? 1));
  const shown = $derived(badges.slice(0, max));
  const folded = $derived(badges.slice(max));
</script>

<div
  role="row"
  class={["row", { selected, dimmed: !enabled, locked, "one-badge": max === 1 }]}
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
    <!-- At large text sizes the name needs the room; the detail panel still shows the category. -->
    {#if mod.category && max > 1}
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
  <!-- Asset-only mods have no real version (the library files them under a placeholder). -->
  <div role="gridcell" class="cell version">{mod.kind === "assets" ? "" : mod.version}</div>
  <div role="gridcell" class="cell status">
    {#each shown as b (b.text)}
      <Badge tone={b.tone}>{b.text}</Badge>
    {/each}
    {#if folded.length > 0}
      {@const more = folded.map((b) => b.text).join(", ")}
      <Tooltip text={more}>
        {#snippet children({ props })}
          <button {...props} type="button" class="more" tabindex={tab} aria-label={`Also: ${more}`}>
            <Badge>+{folded.length}</Badge>
          </button>
        {/snippet}
      </Tooltip>
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
      7ch var(--qm-status-col) var(--qm-row-h);
    /* Two badges with icons, in badge-text units so the column grows with the text size. */
    --qm-status-col: calc(var(--qm-text-11) * 15);
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

  /* One badge plus "+N". */
  .row.one-badge {
    --qm-status-col: calc(var(--qm-text-11) * 10);
  }

  /* The name keeps its room; the category tag shrinks and clips rather than overlapping the version. */
  .name {
    gap: var(--qm-space-2);
    overflow: hidden;
  }

  .name > :global(.badge) {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .title {
    flex: 0 1 auto;
    min-width: 4ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dimmed .title,
  .dimmed .version {
    color: var(--qm-text-muted);
  }

  .moved,
  .more {
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
