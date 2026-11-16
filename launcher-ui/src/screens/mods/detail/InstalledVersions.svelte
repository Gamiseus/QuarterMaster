<script lang="ts">
  import { Button, ConfirmDialog } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type ModSummary } from "../../../lib/bindings";
  import { installs } from "../../../lib/stores/installs.svelte";
  import { mods as modsStore } from "../../../lib/stores/mods.svelte";
  import { toasts } from "../../../lib/stores/toasts.svelte";

  interface Props {
    summary: ModSummary;
  }

  let { summary }: Props = $props();

  let confirmOpen = $state(false);
  let target = $state("");
  const blocked = $derived(installs.blockReason("uninstall"));

  function ask(version: string) {
    target = version;
    confirmOpen = true;
  }

  async function remove() {
    const version = target;
    await call(commands.modVersionRemove(summary.id, version));
    toasts.info(`Removed version ${version} of ${summary.name}.`);
    await modsStore.load();
  }
</script>

<div class="versions" data-testid="installed-versions">
  <span class="label">Installed versions</span>
  <ul aria-label={`Installed versions of ${summary.name}`}>
    {#each summary.installed_versions as v (v)}
      <li>
        <span class="version">{v}</span>
        {#if v === summary.version}
          <span class="qm-muted note">Used by this profile</span>
        {/if}
        <Button size="sm" variant="ghost" disabled={blocked !== null} title={blocked ?? undefined} aria-label={`Remove version ${v}`} onclick={() => ask(v)}>
          Remove
        </Button>
      </li>
    {/each}
  </ul>
  <span class="qm-muted help">Pick the version a profile uses in the mod list. Remove the ones you no longer need.</span>
</div>

<ConfirmDialog
  bind:open={confirmOpen}
  title={`Remove version ${target}?`}
  body={`Version ${target} of ${summary.name} is deleted from the launcher's library. A profile set to use it goes back to the newest installed version. The other versions stay.`}
  confirmLabel="Remove version"
  onconfirm={remove}
/>

<style>
  .versions {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  .label {
    font-size: var(--qm-text-12);
    color: var(--qm-text-muted);
  }

  ul {
    margin: 0;
    padding: 0;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: var(--qm-space-3);
    min-height: 32px;
    padding: 0 var(--qm-space-1) 0 var(--qm-space-2);
  }

  li + li {
    border-top: 1px solid var(--qm-border);
  }

  .version {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  .note {
    flex: 1;
    font-size: var(--qm-text-12);
  }

  li :global(button) {
    margin-left: auto;
  }

  .help {
    font-size: var(--qm-text-12);
  }
</style>
