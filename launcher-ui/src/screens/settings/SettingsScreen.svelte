<script lang="ts">
  import { Screen } from "../../components/ui";
  import AboutCard from "./AboutCard.svelte";
  import AppearanceCard from "./AppearanceCard.svelte";
  import BackupsCard from "./BackupsCard.svelte";
  import FoldersCard from "./FoldersCard.svelte";
  import LaunchCard from "./LaunchCard.svelte";
  import PathsCard from "./PathsCard.svelte";
  import UpdatesCard from "./UpdatesCard.svelte";
</script>

<Screen title="Settings">
  <div class="grid">
    <div class="wide"><PathsCard /></div>
    <LaunchCard />
    <UpdatesCard />
    <BackupsCard />
    <AppearanceCard />
    <FoldersCard />
    <AboutCard />
  </div>
</Screen>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(calc(var(--qm-detail-w) * 1.1), 1fr));
    gap: var(--qm-space-4);
    align-items: start;
  }

  .wide {
    grid-column: 1 / -1;
  }

  /* Selects in setting rows: wide enough for their longest option, not the whole row. */
  .grid :global(.control-select) {
    width: calc(var(--qm-detail-w) * 0.5);
  }

  .grid :global(.settings-card) {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }
</style>
