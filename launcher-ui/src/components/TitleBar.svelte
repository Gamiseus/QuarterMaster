<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";

  // Resolved per click so module load works outside Tauri.
  function win(action: "minimize" | "toggleMaximize" | "close") {
    getCurrentWindow()
      [action]()
      .catch((e: unknown) => console.error(`window ${action} failed`, e));
  }
</script>

<header class="titlebar" data-tauri-drag-region>
  <span class="title" data-tauri-drag-region>QuarterMaster</span>
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
    justify-content: space-between;
    height: 36px;
    padding-left: var(--qm-space-4);
    background: var(--qm-surface);
    border-bottom: 1px solid var(--qm-border);
    user-select: none;
  }

  .title {
    flex: 1;
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.04em;
    color: var(--qm-text-muted);
  }

  .controls {
    display: flex;
    height: 100%;
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
    background: var(--qm-surface-raised);
  }

  .controls button.close:hover {
    background: var(--qm-danger);
    color: #ffffff;
  }

  svg {
    width: 10px;
    height: 10px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1;
  }
</style>
