<script lang="ts">
  import { Button } from "../../../components/ui";
  import type { ModSummary } from "../../../lib/bindings";
  import { installs } from "../../../lib/stores/installs.svelte";
  import { updates } from "../../../lib/stores/updates.svelte";

  interface Props {
    mod: ModSummary;
  }

  let { mod }: Props = $props();

  // Workspace-linked mods are the user's own files: never replaced by a download.
  const update = $derived(mod.is_workspace_linked ? undefined : updates.available.find((u) => u.mod_id === mod.id));
  const reason = $derived(installs.blockReason());
</script>

{#if update}
  <div class="update">
    <Button size="sm" variant="primary" disabled={reason !== null} onclick={() => updates.install(mod.id).catch(() => {})}>
      Update to {update.latest_version}
    </Button>
    {#if reason}
      <span class="qm-muted reason">{reason}</span>
    {/if}
  </div>
{/if}

<style>
  .update {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .reason {
    font-size: var(--qm-text-12);
  }
</style>
