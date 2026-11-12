<script lang="ts">
  import { Badge, Select } from "../../../components/ui";
  import type { FileStat } from "../../../lib/bindings";

  interface Props {
    stats: FileStat[];
    selected: string | null;
    modNames: Record<string, string>;
    onselect: (file: string) => void;
    /** Only files this mod touches ("" for all). */
    modFilter?: string;
  }

  let { stats, selected, modNames, onselect, modFilter = $bindable("") }: Props = $props();

  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const name = (m: string) => modNames[m] ?? m;
  const modItems = $derived([
    { value: "", label: "All mods" },
    ...[...new Set(stats.flatMap((s) => s.mods))].sort((a, b) => name(a).localeCompare(name(b))).map((m) => ({ value: m, label: name(m) })),
  ]);
  const shown = $derived(modFilter ? stats.filter((s) => s.mods.includes(modFilter)) : stats);
</script>

<div class="file-list">
  <Select label="Show files changed by" items={modItems} bind:value={modFilter} />
  {#if shown.length === 0}
    <p class="qm-muted">No files match.</p>
  {:else}
    <ul aria-label="Changed files">
      {#each shown as s (s.target_file)}
        <li>
          <button
            type="button"
            class={["file", { selected: s.target_file === selected }]}
            aria-current={s.target_file === selected ? "true" : undefined}
            onclick={() => onselect(s.target_file)}
          >
            <span class="path">{s.target_file}</span>
            <span class="meta qm-muted">
              {s.original_lines} → {s.patched_lines} lines · {plural(s.patches, "patch", "patches")} · {plural(s.mods.length, "mod", "mods")}
            </span>
            {#if s.conflicts > 0 || s.broken > 0}
              <span class="badges">
                {#if s.conflicts > 0}<Badge tone="warn">{plural(s.conflicts, "conflict", "conflicts")}</Badge>{/if}
                {#if s.broken > 0}<Badge tone="danger">{s.broken} broken</Badge>{/if}
              </span>
            {/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .file-list {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  p {
    margin: 0;
  }

  ul {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .file {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--qm-space-1);
    width: 100%;
    padding: var(--qm-space-2);
    border: 0;
    border-bottom: 1px solid var(--qm-border);
    border-left: 3px solid transparent;
    background: none;
    color: var(--qm-text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .file:hover {
    background: var(--qm-raised);
  }

  .file.selected {
    border-left-color: var(--qm-accent);
    background: var(--qm-raised);
  }

  .file:focus-visible {
    outline: none;
    box-shadow: inset var(--qm-focus-ring);
  }

  .path {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    overflow-wrap: anywhere;
  }

  .meta {
    font-size: var(--qm-text-11);
  }

  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-1);
  }
</style>
