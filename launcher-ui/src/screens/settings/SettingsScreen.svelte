<script lang="ts">
  import { Screen } from "../../components/ui";
  import AboutCard from "./AboutCard.svelte";
  import AppearanceCard from "./AppearanceCard.svelte";
  import BackupsCard from "./BackupsCard.svelte";
  import FoldersCard from "./FoldersCard.svelte";
  import GameWindowCard from "./GameWindowCard.svelte";
  import LaunchCard from "./LaunchCard.svelte";
  import PathsCard from "./PathsCard.svelte";
  import StorageCard from "./StorageCard.svelte";
  import UpdatesCard from "./UpdatesCard.svelte";
</script>

<Screen title="Settings">
  <!-- The cards flow into as many columns as fit (three at most), each column stacking its own
       cards, so a tall card leaves no gap beside it and a wide window leaves no empty side. -->
  <div class="settings">
    <PathsCard />
    <div class="cards">
      <LaunchCard />
      <GameWindowCard />
      <BackupsCard />
      <FoldersCard />
      <StorageCard />
      <UpdatesCard />
      <AppearanceCard />
      <AboutCard />
    </div>
  </div>
</Screen>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-4);
  }

  /* A column is at least 35 lines of body text wide, so a row keeps its control beside its text
     and a larger text size gets fewer, wider columns. */
  .cards {
    columns: calc(var(--qm-text-13) * 35) 3;
    column-gap: var(--qm-space-4);
  }

  .cards > :global(*) {
    break-inside: avoid;
    margin-bottom: var(--qm-space-4);
  }

  /* Selects in setting rows: wide enough for their longest option, not the whole row. */
  .settings :global(.control-select) {
    width: calc(var(--qm-detail-w) * 0.5);
  }

  .settings :global(.settings-card) {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }
</style>
