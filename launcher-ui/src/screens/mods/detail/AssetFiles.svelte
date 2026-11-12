<script lang="ts">
  import { Badge, Section } from "../../../components/ui";
  import type { AssetFile } from "../../../lib/bindings";
  import { assetSummary, groupAssetFiles } from "../../../lib/modlist/assets";

  interface Props {
    files: AssetFile[];
  }

  let { files }: Props = $props();

  /** Folders with more files than this start collapsed. */
  const OPEN_LIMIT = 40;

  const groups = $derived(groupAssetFiles(files));
  const unknown = $derived(files.some((f) => f.overrides_vanilla === null));
</script>

<Section title="Files" description={assetSummary(files)}>
  {#if unknown}
    <p class="qm-muted note">Which files replace the game's own shows once the game files are read.</p>
  {/if}
  {#each groups as g (g.folder)}
    <details class="group" open={g.files.length <= OPEN_LIMIT}>
      <summary><span class="folder">{g.folder === "" ? "assets" : `assets/${g.folder}`}</span> <span class="qm-muted">{g.files.length}</span></summary>
      <ul aria-label={`Files in ${g.folder === "" ? "assets" : g.folder}`}>
        {#each g.files as f (f.path)}
          <li>
            <span class="name">{f.name}</span>
            {#if f.overrides === true}
              <!-- Plain tones: replacing the game's files is what these mods are for, not a warning. -->
              <Badge tone="accent">Overrides</Badge>
            {:else if f.overrides === false}
              <Badge>Adds</Badge>
            {/if}
          </li>
        {/each}
      </ul>
    </details>
  {/each}
</Section>

<style>
  .note {
    margin: 0 0 var(--qm-space-2);
  }

  .group + .group {
    margin-top: var(--qm-space-2);
  }

  summary {
    cursor: pointer;
  }

  .folder {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  ul {
    margin: var(--qm-space-1) 0 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-2);
    min-height: 28px;
    padding: 0 var(--qm-space-2);
    border-bottom: 1px solid var(--qm-border);
  }

  .name {
    min-width: 0;
    overflow-wrap: anywhere;
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }
</style>
