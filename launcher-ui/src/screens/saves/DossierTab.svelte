<script lang="ts">
  import { EmptyState, KeyValueList, Section } from "../../components/ui";
  import type { SaveProfile, SaveSummary } from "../../lib/bindings";
  import { isEmptySave, modeSections, squadRows } from "../../lib/saves/dossier";

  interface Props {
    summary: SaveSummary;
    profile: SaveProfile;
  }

  let { summary, profile }: Props = $props();

  const sections = $derived(modeSections(summary));
  const squad = $derived(squadRows(summary));
  const mods = $derived(
    Object.entries(profile.active_mods_fingerprint ?? {})
      .filter((e): e is [string, string] => e[1] !== undefined)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([id, version]) => ({ label: id, value: version, mono: true })),
  );
</script>

<div class="dossier">
  {#if isEmptySave(summary)}
    <EmptyState title="No progress yet." body="Play a game in this slot and its dossier fills in." />
  {:else}
    <!-- One section per game mode the save has progress in; the save keeps them apart. -->
    {#each sections as sec (sec.mode)}
      <Section title={sec.title}>
        <KeyValueList label="{sec.title} figures" items={sec.rows} />
      </Section>
    {/each}
    <Section title="Squad">
      <KeyValueList label="Squad figures" items={squad} />
    </Section>
  {/if}

  <Section title="Mods last used">
    {#if mods.length > 0}
      <KeyValueList label="Mods last used with this slot" items={mods} />
    {:else}
      <p class="qm-muted none">No mods recorded for this slot.</p>
    {/if}
  </Section>
</div>

<style>
  .dossier {
    display: flex;
    flex-direction: column;
    gap: var(--qm-gap-section);
    container-type: inline-size;
  }

  /* Wide enough for two columns of figures: use them rather than one long list. */
  @container (min-width: 760px) {
    .dossier :global(.qm-kv) {
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      column-gap: var(--qm-space-6);
    }

    .dossier :global(.qm-kv .row:nth-last-child(2):nth-child(odd)) {
      border-bottom: 0;
    }
  }

  .none {
    margin: 0;
    font-size: var(--qm-text-13);
  }
</style>
