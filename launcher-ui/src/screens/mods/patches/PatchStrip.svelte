<script lang="ts">
  import { Badge } from "../../../components/ui";
  import type { PatchExecutionRecord, PatchOutcomeStatus } from "../../../lib/bindings";
  import { modColours } from "../../../lib/stores/modcolours.svelte";
  import { isShadowed } from "../../../lib/patches/explain";

  interface Props {
    /** The file's patch records, in application order. */
    records: PatchExecutionRecord[];
    selected: string | null;
    modNames: Record<string, string>;
    onselect: (patchId: string) => void;
    /** The mods the colours are assigned over (the diff view's set, so the colours match). */
    colourMods?: string[];
  }

  let { records, selected, modNames, onselect, colourMods }: Props = $props();

  const STATUS: Record<PatchOutcomeStatus, { label: string; tone: "ok" | "danger" | "neutral" }> = {
    applied: { label: "Applied", tone: "ok" },
    zero_matches: { label: "Broken", tone: "danger" },
    count_mismatch: { label: "Broken", tone: "danger" },
    aborted: { label: "Broken", tone: "danger" },
    skipped_by_policy: { label: "Skipped", tone: "neutral" },
    skipped_disabled: { label: "Disabled", tone: "neutral" },
  };

  /** A module replaces the file, so the game never runs its patches (G1). */
  const NOT_RUN = { label: "Does not run in the game", tone: "warn" } as const;
  const badgeOf = (r: PatchExecutionRecord) => (r.status !== "skipped_disabled" && isShadowed(r) ? NOT_RUN : STATUS[r.status]);

  const colours = $derived(modColours(colourMods ?? records.map((r) => r.mod_id)));

  let strip = $state<HTMLElement>();
  /** The strip only scrolls sideways, so the wheel moves it sideways without Shift. */
  function onwheel(e: WheelEvent) {
    if (!strip || e.deltaX !== 0 || e.deltaY === 0 || strip.scrollWidth <= strip.clientWidth) return;
    e.preventDefault();
    strip.scrollLeft += e.deltaY;
  }
  // The picked patch is brought fully into view (a click on one half past the edge, a deep link).
  $effect(() => {
    const id = selected;
    void records;
    if (!id || !strip) return;
    const chip = strip.querySelector<HTMLElement>(`[data-patch="${CSS.escape(id)}"]`);
    chip?.scrollIntoView?.({ block: "nearest", inline: "nearest", behavior: "smooth" });
  });
</script>

<ol
  class="strip"
  aria-label="Patches in application order"
  title="Patches in this file, in the order they run. Pick one to jump to its change and see what it does."
  bind:this={strip}
  {onwheel}
>
  {#each records as r, i (r.patch_id)}
    <li>
      <button
        type="button"
        class={["chip", `c${colours.get(r.mod_id) ?? 0}`, { selected: r.patch_id === selected }]}
        aria-pressed={r.patch_id === selected}
        data-patch={r.patch_id}
        onclick={() => onselect(r.patch_id)}
      >
        <span class="n">{i + 1}</span>
        <span class="what">
          <span class="name">{r.patch_name || r.patch_id}</span>
          <span class="sub">
            <span class="mod qm-muted">{modNames[r.mod_id] ?? r.mod_id}</span>
            <!-- The group its author filed it under, when it has one. -->
            {#if r.group}<span class="group qm-muted" data-testid="chip-group">{r.group}</span>{/if}
            <Badge tone={badgeOf(r).tone}>{badgeOf(r).label}</Badge>
          </span>
        </span>
      </button>
    </li>
  {/each}
</ol>

<style>
  .strip {
    display: flex;
    gap: var(--qm-space-2);
    margin: 0;
    padding: 0 0 var(--qm-space-1);
    overflow-x: auto;
    list-style: none;
  }

  /* Two lines: the patch's name, then its mod and state. A name longer than the chip is cut;
     the details and the change itself show it whole. */
  .chip {
    display: flex;
    align-items: center;
    gap: var(--qm-space-3);
    max-width: calc(var(--qm-text-13) * 30);
    padding: var(--qm-space-2) var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-left: 4px solid var(--attr, var(--qm-border-strong));
    border-radius: var(--qm-radius-sm);
    background: var(--qm-panel);
    color: var(--qm-text);
    font: inherit;
    font-size: var(--qm-text-13);
    text-align: left;
    white-space: nowrap;
    cursor: pointer;
  }

  .what {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    min-width: 0;
  }

  .sub {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
    font-size: var(--qm-text-12);
  }

  .group::before {
    margin-right: var(--qm-space-2);
    content: "/";
  }

  .chip.selected {
    border-color: var(--qm-accent);
    border-left-color: var(--attr, var(--qm-accent));
    background: var(--qm-raised);
  }

  .chip:focus-visible {
    outline: none;
    box-shadow: var(--qm-focus-ring);
  }

  .n {
    color: var(--qm-text-muted);
    font-family: var(--qm-font-mono);
  }

  .name {
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
  }

  .c1 { --attr: var(--qm-attr-1); }
  .c2 { --attr: var(--qm-attr-2); }
  .c3 { --attr: var(--qm-attr-3); }
  .c4 { --attr: var(--qm-attr-4); }
  .c5 { --attr: var(--qm-attr-5); }
  .c6 { --attr: var(--qm-attr-6); }
  .c7 { --attr: var(--qm-attr-7); }
  .c8 { --attr: var(--qm-attr-8); }
</style>
