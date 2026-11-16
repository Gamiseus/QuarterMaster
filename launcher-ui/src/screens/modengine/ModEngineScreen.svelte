<script lang="ts">
  import { Button, EmptyState, Panel, Screen } from "../../components/ui";
  import { app } from "../../lib/stores/app.svelte";

  function backToMods() {
    app.mode = "launcher";
    app.page = "mods";
  }
</script>

<Screen title="Mod Engine">
  <Panel class="soon">
    <EmptyState title="Tools for making mods" body="Mod Engine arrives in a later update.">
      {#snippet action()}
        <Button onclick={backToMods}>Back to Mods</Button>
      {/snippet}
    </EmptyState>
  </Panel>
</Screen>

<style>
  :global(.soon) {
    max-width: calc(var(--qm-detail-w) * 1.6);
    margin: var(--qm-space-6) auto 0;
  }
</style>
