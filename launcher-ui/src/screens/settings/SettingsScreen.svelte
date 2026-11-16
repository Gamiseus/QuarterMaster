<script lang="ts">
  import { Screen } from "../../components/ui";
  import AboutCard from "./AboutCard.svelte";
  import AppearanceCard from "./AppearanceCard.svelte";
  import BackupsCard from "./BackupsCard.svelte";
  import FoldersCard from "./FoldersCard.svelte";
  import GameWindowCard from "./GameWindowCard.svelte";
  import LaunchCard from "./LaunchCard.svelte";
  import PathsCard from "./PathsCard.svelte";
  import UpdatesCard from "./UpdatesCard.svelte";
</script>

<Screen title="Settings">
  <!-- Three groups on one grid of up to three columns. Cards in a row are equally tall, so every
       row starts and ends on one line; a card that spans columns lays its rows out in columns. -->
  <div class="settings">
    <section class="group" aria-labelledby="settings-game">
      <h2 class="group-title" id="settings-game">Game</h2>
      <div class="grid game">
        <div class="cell paths"><PathsCard /></div>
        <div class="cell launch"><LaunchCard /></div>
        <div class="cell window"><GameWindowCard /></div>
      </div>
    </section>
    <section class="group" aria-labelledby="settings-mods">
      <h2 class="group-title" id="settings-mods">Mods and saves</h2>
      <div class="grid mods">
        <div class="cell updates"><UpdatesCard /></div>
        <div class="cell backups"><BackupsCard /></div>
        <div class="cell folders"><FoldersCard /></div>
      </div>
    </section>
    <section class="group" aria-labelledby="settings-app">
      <h2 class="group-title" id="settings-app">QuarterMaster</h2>
      <div class="grid app">
        <div class="cell appearance"><AppearanceCard /></div>
        <div class="cell about"><AboutCard /></div>
      </div>
    </section>
  </div>
</Screen>

<style>
  .settings {
    container: settings / inline-size;
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-5);
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  /* A small stencil label with a rule running to the edge. */
  .group-title {
    display: flex;
    align-items: center;
    gap: var(--qm-space-3);
    color: var(--qm-text-muted);
    font-size: var(--qm-text-13);
  }

  .group-title::after {
    flex: 1;
    height: 1px;
    background: var(--qm-border);
    content: "";
  }

  .grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--qm-space-4);
  }

  .cell {
    display: grid;
    min-width: 0;
  }

  .paths { grid-area: paths; }
  .launch { grid-area: launch; }
  .window { grid-area: window; }
  .updates { grid-area: updates; }
  .backups { grid-area: backups; }
  .folders { grid-area: folders; }
  .appearance { grid-area: appearance; }
  .about { grid-area: about; }

  .game { grid-template-areas: "paths" "launch" "window"; }
  .mods { grid-template-areas: "updates" "backups" "folders"; }
  .app { grid-template-areas: "appearance" "about"; }

  @container settings (min-width: 860px) {
    .grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .game { grid-template-areas: "paths paths" "launch launch" "window window"; }
    .mods { grid-template-areas: "updates folders" "backups backups"; }
    .app { grid-template-areas: "appearance appearance" "about about"; }
  }

  @container settings (min-width: 1400px) {
    .grid {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }

    .game { grid-template-areas: "paths paths launch" "window window window"; }
    .mods { grid-template-areas: "updates backups folders"; }
    .app { grid-template-areas: "appearance appearance about"; }
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

  /* Rows of a card that spans columns: as many columns of rows as fit, each row keeping its line. */
  .settings :global(.settings-fields) {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 480px), 1fr));
    column-gap: var(--qm-space-5);
  }

  .settings :global(.settings-fields > .qm-field-row:last-child) {
    border-bottom: 1px solid var(--qm-border);
  }
</style>
