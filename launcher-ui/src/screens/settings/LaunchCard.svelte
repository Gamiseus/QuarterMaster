<script lang="ts">
  import { FieldRow, Panel, Select, StencilHeading, Switch } from "../../components/ui";
  import type { LaunchMethod, SettingsPatch } from "../../lib/bindings";
  import { app } from "../../lib/stores/app.svelte";

  const METHODS: { value: LaunchMethod; label: string }[] = [
    { value: "steam_url", label: "Steam" },
    { value: "direct_exe", label: "Direct" },
  ];

  const s = $derived(app.settings);
  const disabled = $derived(!app.startupDone || !s);

  function save(patch: Partial<SettingsPatch>) {
    app.updateSettings(patch).catch(() => {});
  }
</script>

<Panel class="settings-card">
  <StencilHeading level={2}>Launch</StencilHeading>
  <div>
    <FieldRow label="Start the game with" help="Steam starts it through the Steam client. Direct runs mnb.exe.">
      <Select
        class="control-select"
        label="Start the game with"
        hideLabel
        items={METHODS}
        {disabled}
        bind:value={() => s?.default_launch_method, (v) => v && v !== s?.default_launch_method && save({ default_launch_method: v })}
      />
    </FieldRow>
    <!-- Both apply to every profile. The console window opens only in debug mode. -->
    <FieldRow
      label="Debug mode"
      help="For mod makers. Opens the console window next to the game, dumps every file the game loads and turns on the dump keys. The log file is written either way."
    >
      <Switch
        label="Debug mode"
        hideLabel
        {disabled}
        bind:checked={() => s?.debug_mode ?? false, (v) => save({ debug_mode: v })}
      />
    </FieldRow>
    <FieldRow
      label="Developer commands"
      help="Unlocks the game's own developer keys: F1 shows the AI overlay, F5 turns on the dev blast (right-click calls an air strike)."
    >
      <Switch
        label="Developer commands"
        hideLabel
        {disabled}
        bind:checked={() => s?.dev_commands ?? false, (v) => save({ dev_commands: v })}
      />
    </FieldRow>
  </div>
</Panel>
