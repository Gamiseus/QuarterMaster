<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import iconUrl from "../assets/app-icon.svg";
  import ProfilePicker from "../screens/profiles/ProfilePicker.svelte";
  import HealthSummary from "./HealthSummary.svelte";
  import LaunchButton from "./LaunchButton.svelte";
  import SaveSlotChip from "./SaveSlotChip.svelte";

  // Resolved per click so module load works outside Tauri.
  function win(action: "minimize" | "toggleMaximize" | "close") {
    getCurrentWindow()
      [action]()
      .catch((e: unknown) => console.error(`window ${action} failed`, e));
  }
</script>

<header class="titlebar" data-tauri-drag-region>
  <img class="icon" src={iconUrl} alt="" data-tauri-drag-region />
  <ProfilePicker />
  <SaveSlotChip />
  <span class="spacer" data-tauri-drag-region></span>
  <HealthSummary />
  <LaunchButton />
  <div class="controls">
    <button type="button" aria-label="Minimize" onclick={() => win("minimize")}>
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M1 5h8" /></svg>
    </button>
    <button type="button" aria-label="Maximize" onclick={() => win("toggleMaximize")}>
      <svg viewBox="0 0 10 10" aria-hidden="true"><rect x="1.5" y="1.5" width="7" height="7" /></svg>
    </button>
    <button type="button" class="close" aria-label="Close" onclick={() => win("close")}>
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M1.5 1.5l7 7M8.5 1.5l-7 7" /></svg>
    </button>
  </div>
</header>

<style>
  .titlebar {
    display: flex;
    align-items: center;
    gap: var(--qm-space-3);
    height: var(--qm-titlebar-h);
    padding-left: var(--qm-space-3);
    background: var(--qm-surface);
    border-bottom: 1px solid var(--qm-border);
    user-select: none;
  }

  .icon {
    flex: none;
    width: var(--qm-space-5);
    height: var(--qm-space-5);
  }

  .spacer {
    flex: 1;
    align-self: stretch;
  }

  .controls {
    display: flex;
    align-self: stretch;
    margin-left: var(--qm-space-2);
  }

  .controls button {
    display: grid;
    place-items: center;
    width: 46px;
    height: 100%;
    border: 0;
    background: transparent;
    transition: background var(--qm-dur-fast) var(--qm-ease);
  }

  .controls button:hover {
    background: var(--qm-raised);
  }

  .controls button.close:hover {
    background: var(--qm-danger);
    color: var(--qm-accent-contrast);
  }

  svg {
    width: 10px;
    height: 10px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1;
  }
</style>
