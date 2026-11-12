<script lang="ts">
  import { KeyValueList, Panel, StencilHeading } from "../../components/ui";
  import { app } from "../../lib/stores/app.svelte";

  const info = $derived(app.info);
  const gameVersion = $derived(info?.game.status === "ready" && info.game.data ? info.game.data.version : "Not detected");
  const rows = $derived(
    info
      ? [
          { label: "Version", value: info.version },
          { label: "Install", value: info.is_portable ? "Portable" : "Installed" },
          { label: "Game version", value: gameVersion },
          { label: "Data folder", value: info.data_dir, mono: true, copy: true },
        ]
      : [],
  );
</script>

<Panel class="settings-card">
  <StencilHeading level={2}>About</StencilHeading>
  <KeyValueList label="About QuarterMaster" items={rows} />
</Panel>
