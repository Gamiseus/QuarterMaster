<script lang="ts">
  import { Select } from "../../../components/ui";
  import { app } from "../../../lib/stores/app.svelte";
  import { updates, type AutoUpdateChoice } from "../../../lib/stores/updates.svelte";

  interface Props {
    modId: string;
  }

  let { modId }: Props = $props();

  const CHOICES: { value: AutoUpdateChoice; label: string }[] = [
    { value: "default", label: "Use default" },
    { value: "always", label: "Always" },
    { value: "never", label: "Never" },
  ];

  const choice = $derived(updates.choiceFor(modId));
  const globalAuto = $derived(app.settings?.auto_install_updates ?? false);

  function pick(v: string) {
    const next = CHOICES.find((c) => c.value === v)?.value;
    if (next && next !== choice) updates.setChoice(modId, next).catch(() => {});
  }
</script>

<!-- This mod's own choice; "Use default" follows Settings > Updates. -->
<div class="auto">
  <Select
    class="auto-select"
    label="Update this mod automatically"
    items={CHOICES}
    disabled={!app.startupDone || !app.settings}
    bind:value={() => choice, pick}
  />
  {#if choice === "default"}
    <span class="qm-muted hint">Default in Settings: {globalAuto ? "install automatically" : "ask first"}.</span>
  {/if}
</div>

<style>
  .auto {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--qm-space-2) var(--qm-space-3);
  }

  .auto :global(.auto-select) {
    width: calc(var(--qm-detail-w) * 0.6);
  }

  .hint {
    padding-bottom: var(--qm-space-2);
    font-size: var(--qm-text-12);
  }
</style>
