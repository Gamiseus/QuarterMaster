<script lang="ts">
  import { tick } from "svelte";
  import { IconChevronLeft, IconChevronRight, IconLock, IconPlus } from "../../components/icons";
  import { Badge, IconButton, Skeleton } from "../../components/ui";
  import { formatTimestamp } from "../../lib/saves/format";
  import { slotTags } from "../../lib/saves/tags";
  import { app } from "../../lib/stores/app.svelte";
  import { saves } from "../../lib/stores/saves.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";
  import NameDialog from "../profiles/NameDialog.svelte";

  let strip = $state<HTMLElement | null>(null);
  let canBack = $state(false);
  let canForward = $state(false);
  let newOpen = $state(false);

  const list = $derived(saves.list ?? []);
  const current = $derived(saves.current);

  function measure() {
    if (!strip) return;
    canBack = strip.scrollLeft > 1;
    canForward = strip.scrollLeft + strip.clientWidth < strip.scrollWidth - 1;
  }

  // Re-measure when the slots change or the strip resizes.
  $effect(() => {
    void list.length;
    if (!strip) return;
    const ro = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    ro?.observe(strip);
    void tick().then(measure);
    return () => ro?.disconnect();
  });

  // Keep the selected card in view.
  $effect(() => {
    const id = current;
    if (!strip || !id) return;
    void tick().then(() => {
      strip?.querySelector<HTMLElement>(`[data-slot="${CSS.escape(id)}"]`)?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
    });
  });

  function pick(id: string, focus = false) {
    saves.selected = id;
    if (focus) void tick().then(() => strip?.querySelector<HTMLElement>(`[data-slot="${CSS.escape(id)}"]`)?.focus());
  }

  function onkeydown(e: KeyboardEvent) {
    const i = list.findIndex((s) => s.profile.id === current);
    let next = -1;
    if (e.key === "ArrowRight") next = Math.min(list.length - 1, i + 1);
    else if (e.key === "ArrowLeft") next = Math.max(0, i - 1);
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = list.length - 1;
    if (next < 0 || next === i) return;
    e.preventDefault();
    pick(list[next].profile.id, true);
  }

  /** A vertical wheel scrolls the strip sideways. */
  function onwheel(e: WheelEvent) {
    if (!strip || Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
    if (strip.scrollWidth <= strip.clientWidth) return;
    e.preventDefault();
    strip.scrollLeft += e.deltaY;
  }

  function page(dir: 1 | -1) {
    strip?.scrollBy({ left: dir * strip.clientWidth * 0.8, behavior: "smooth" });
  }

  async function create(name: string) {
    await saves.create(name);
    toasts.info(`Created ${name}.`);
  }
</script>

<div class="wrap">
  {#if canBack}
    <IconButton class="nav back" icon={IconChevronLeft} label="Scroll the slots left" onclick={() => page(-1)} />
  {/if}
  {#if saves.list === null}
    <div class="strip" aria-busy="true" aria-label="Loading save slots"><Skeleton lines={2} /></div>
  {:else}
    <div class="strip" bind:this={strip} onscroll={measure} {onwheel}>
      <!-- svelte-ignore a11y_interactive_supports_focus (the options carry the roving tab stop) -->
      <div class="cards" role="listbox" aria-label="Save slots" aria-orientation="horizontal" tabindex="-1" {onkeydown}>
        {#each list as slot (slot.profile.id)}
          {@const id = slot.profile.id}
          {@const selected = current === id}
          <div
            role="option"
            class={["card", { selected }]}
            aria-selected={selected}
            tabindex={selected ? 0 : -1}
            data-slot={id}
            onclick={() => pick(id)}
            onkeydown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), pick(id))}
          >
            <span class="name">{slot.profile.name}</span>
            <span class="tags">
              {#if slot.is_active}<Badge tone="accent">Active</Badge>{/if}
              {#each slotTags(slot.profile) as tag (tag.label)}
                <Badge tone={tag.tone} icon={false}
                  >{#if tag.label === "Locked"}<IconLock class="lock" />{/if}{tag.label}</Badge
                >
              {/each}
            </span>
            <span class="qm-muted when">
              {slot.profile.last_played_at
                ? `Played ${formatTimestamp(slot.profile.last_played_at)}`
                : `Created ${formatTimestamp(slot.profile.created_at)}`}
            </span>
          </div>
        {/each}
      </div>
      <button type="button" class="card new" disabled={!app.startupDone} onclick={() => (newOpen = true)}>
        <IconPlus />
        <span>New slot</span>
      </button>
    </div>
  {/if}
  {#if canForward}
    <IconButton class="nav forward" icon={IconChevronRight} label="Scroll the slots right" onclick={() => page(1)} />
  {/if}
</div>

<NameDialog
  bind:open={newOpen}
  title="New save slot"
  label="Slot name"
  submitLabel="Create"
  description="An empty slot: the game starts fresh in it."
  disabled={!app.startupDone}
  onsubmit={create}
/>

<style>
  .wrap {
    position: relative;
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--qm-space-2);
    min-width: 0;
  }

  .strip {
    display: flex;
    flex: 1;
    gap: var(--qm-space-2);
    min-width: 0;
    padding: 2px 2px var(--qm-space-2);
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: thin;
  }

  .cards {
    display: flex;
    gap: var(--qm-space-2);
  }

  .card {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: var(--qm-space-1);
    width: 220px;
    padding: var(--qm-space-2) var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-md);
    background: var(--qm-panel);
    color: var(--qm-text);
    text-align: left;
    cursor: pointer;
    transition: border-color var(--qm-dur-fast) var(--qm-ease);
  }

  .card:hover {
    border-color: var(--qm-border-strong);
  }

  .card.selected {
    border-color: var(--qm-accent);
    background: color-mix(in srgb, var(--qm-accent) 12%, var(--qm-panel));
  }

  .card:focus-visible {
    box-shadow: var(--qm-focus-ring);
  }

  .name {
    overflow: hidden;
    font-size: var(--qm-text-14);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-1);
  }

  .tags :global(.lock) {
    width: 1em;
    height: 1em;
    margin-right: 2px;
  }

  .when {
    font-size: var(--qm-text-12);
  }

  .new {
    align-items: center;
    justify-content: center;
    flex-direction: row;
    gap: var(--qm-space-2);
    width: 140px;
    border-style: dashed;
    color: var(--qm-text-muted);
    font-weight: 600;
  }

  .new:disabled {
    cursor: default;
    opacity: 0.6;
  }
</style>
