<script lang="ts">
  import { Button, FieldRow, Panel, StencilHeading } from "../../components/ui";
  import type { RevealKind } from "../../lib/bindings";
  import { app } from "../../lib/stores/app.svelte";

  // Fixed kinds only: the backend resolves each to its folder, so no path ever comes from the UI.
  const FOLDERS: { kind: RevealKind; label: string; name: string }[] = [
    { kind: "logs", label: "Logs", name: "logs" },
    { kind: "library", label: "Mod library", name: "mod library" },
    { kind: "data_dir", label: "QuarterMaster data", name: "data" },
    { kind: "game_dir", label: "Game", name: "game" },
    { kind: "save_dir", label: "Saves", name: "save" },
    { kind: "profile_dir", label: "Active profile", name: "profile" },
  ];
</script>

<Panel class="settings-card">
  <StencilHeading level={2}>Folders</StencilHeading>
  <ul class="list" aria-label="Folders">
    {#each FOLDERS as f (f.kind)}
      <li>
        <FieldRow label={f.label}>
          <Button size="sm" aria-label="Open the {f.name} folder" onclick={() => app.reveal(f.kind).catch(() => {})}>
            Open
          </Button>
        </FieldRow>
      </li>
    {/each}
  </ul>
</Panel>

<style>
  .list {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* The FieldRow divider sits between rows, not under the last one. */
  .list li:last-child :global(.qm-field-row) {
    border-bottom: 0;
  }
</style>
