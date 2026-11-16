<script lang="ts">
  import { Button, KeyValueList, Panel, StencilHeading } from "../../components/ui";
  import { openExternal } from "../../lib/external";
  import { setup } from "../../lib/stores/setup.svelte";
  import { app } from "../../lib/stores/app.svelte";

  const info = $derived(app.info);
  const gameVersion = $derived(info?.game.status === "ready" && info.game.data ? info.game.data.version : "Not detected");
  const rows = $derived(
    info
      ? [
          { label: "Version", value: info.version },
          { label: "Data", value: info.is_portable ? "Beside the program" : "In AppData" },
          { label: "Game version", value: gameVersion },
        ]
      : [],
  );
</script>

<Panel class="settings-card">
  <StencilHeading level={2}>About</StencilHeading>
  <KeyValueList label="About QuarterMaster" items={rows} />
  <div class="links">
    <Button size="sm" onclick={() => (setup.reportOpen = true)}>Report a problem...</Button>
    <Button size="sm" variant="ghost" onclick={() => void openExternal("https://gamiseus.github.io/")}>Website and wiki</Button>
    <Button size="sm" variant="ghost" onclick={() => void openExternal("https://github.com/Gamiseus/QuarterMaster")}>GitHub</Button>
  </div>
</Panel>

<style>
  .links {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-2);
    margin-top: var(--qm-space-3);
  }
</style>
