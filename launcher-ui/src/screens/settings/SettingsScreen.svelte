<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { app } from "../../lib/stores/app.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";

  async function browse() {
    let path: string | null;
    try {
      path = await open({ filters: [{ name: "mnb.exe", extensions: ["exe"] }] });
    } catch (e) {
      toasts.error("Could not open the file picker.", e instanceof Error ? e.message : String(e));
      return;
    }
    if (!path) return;
    await app.updateSettings({ game_exe_path: path }).catch(() => {});
  }
</script>

<section class="qm-page">
  <h1>Settings</h1>

  <div class="qm-card">
    <h2>Game</h2>
    <div class="row">
      <div class="path">
        <p class="qm-muted">Game executable</p>
        <code data-testid="game-path">{app.settings?.game_exe_path ?? "Not set"}</code>
      </div>
      <button type="button" class="qm-btn" onclick={browse}>Browse</button>
    </div>
  </div>
</section>

<style>
  .row {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--qm-space-4);
    margin-top: var(--qm-space-3);
  }

  .path {
    min-width: 0;
  }

  p {
    margin: 0 0 var(--qm-space-1);
  }

  code {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
