<script lang="ts">
  import { Checkbox, Inset, RadioGroup } from "../../../components/ui";
  import type { ArchiveModCandidate } from "../../../lib/bindings";
  import { choicesFor, type Choice } from "../../../lib/install/plan";

  interface Props {
    candidates: ArchiveModCandidate[];
    /** Mod id to whether it is installed from this archive. */
    checked: Record<string, boolean>;
    /** Mod id to what happens to the installed copy (installed mods only). */
    choices: Record<string, Choice>;
    gameBusy: boolean;
  }

  let { candidates, checked = $bindable(), choices = $bindable(), gameBusy }: Props = $props();
</script>

<ul class="list" aria-label="Mods in this archive">
  {#each candidates as c (c.manifest.id)}
    {@const id = c.manifest.id}
    {@const items = choicesFor(c, gameBusy)}
    <li>
      <Inset class="item">
        <Checkbox bind:checked={checked[id]} label="{c.manifest.name} {c.manifest.version}" />
        {#if c.all_installed_versions.length > 0}
          <span class="qm-muted installed">Installed: {c.all_installed_versions.join(", ")}</span>
        {/if}
        {#if items.length > 0 && checked[id]}
          <RadioGroup class="choice" bind:value={choices[id]} {items} label="{c.manifest.name} is already installed" />
        {/if}
      </Inset>
    </li>
  {/each}
</ul>

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* Each mod in the archive is its own inset box, so its choices never blur into the next mod. */
  .list :global(.item) {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .installed {
    font-size: var(--qm-text-12);
  }

  .list :global(.item .choice) {
    padding-left: var(--qm-space-5);
  }
</style>
