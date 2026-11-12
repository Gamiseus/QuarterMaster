<script lang="ts">
  import { Badge } from "../../../components/ui";
  import type { PatchExecutionRecord, PatchOutcomeStatus } from "../../../lib/bindings";
  import { assignModColours } from "../../../lib/patches/colors";
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

  const colours = $derived(assignModColours(colourMods ?? records.map((r) => r.mod_id)));
</script>

<ol class="strip" aria-label="Patches in application order">
  {#each records as r, i (r.patch_id)}
    <li>
      <button
        type="button"
        class={["chip", `c${colours.get(r.mod_id) ?? 0}`, { selected: r.patch_id === selected }]}
        aria-pressed={r.patch_id === selected}
        onclick={() => onselect(r.patch_id)}
      >
        <span class="n">{i + 1}</span>
        <span class="name">{r.patch_name || r.patch_id}</span>
        <span class="mod qm-muted">{modNames[r.mod_id] ?? r.mod_id}</span>
        <Badge tone={badgeOf(r).tone}>{badgeOf(r).label}</Badge>
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

  .chip {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
    padding: var(--qm-space-1) var(--qm-space-2);
    border: 1px solid var(--qm-border);
    border-left: 4px solid var(--attr, var(--qm-border-strong));
    border-radius: var(--qm-radius-sm);
    background: var(--qm-panel);
    color: var(--qm-text);
    font: inherit;
    font-size: var(--qm-text-12);
    white-space: nowrap;
    cursor: pointer;
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
    font-weight: 600;
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
