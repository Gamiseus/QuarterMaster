<script lang="ts">
  import { call } from "../../lib/api";
  import { loadOrder } from "../../lib/stores/loadorder.svelte";
  import { commands, type SwitchPreview } from "../../lib/bindings";
  import { Button, Dialog, StencilHeading } from "../../components/ui";
  import { app } from "../../lib/stores/app.svelte";
  import { mods } from "../../lib/stores/mods.svelte";
  import { prelaunch } from "../../lib/stores/prelaunch.svelte";
  import { profiles } from "../../lib/stores/profiles.svelte";

  interface Props {
    open?: boolean;
    /** Profile to switch to. */
    profileId: string | null;
  }

  let { open = $bindable(false), profileId }: Props = $props();

  let preview = $state<SwitchPreview | null>(null);
  let failed = $state(false);
  let switching = $state(false);
  let seq = 0;

  const target = $derived(profiles.list.find((p) => p.id === profileId) ?? null);
  const warning = $derived(preview?.save_compatibility.warning_message ?? null);
  const unchanged = $derived(preview !== null && preview.added_mods.length === 0 && preview.removed_mods.length === 0);

  function modName(id: string): string {
    if (mods.list.status !== "ready") return id;
    return mods.list.data.find((m) => m.id === id)?.name ?? id;
  }

  // Load the preview each time the dialog opens for a profile.
  $effect(() => {
    const id = profileId;
    if (!open || !id) return;
    const mine = ++seq;
    preview = null;
    failed = false;
    call(commands.profilePreviewSwitch(id))
      .then((p) => {
        if (mine === seq) preview = p;
      })
      .catch(() => {
        if (mine === seq) failed = true;
      });
  });

  async function confirm() {
    if (!profileId || switching) return;
    switching = true;
    try {
      // The new profile's resolution must land before Launch can pass the error gate again.
      const id = profileId;
      await loadOrder.whileChanging(() => call(commands.profileActivate(id)));
    } catch {
      // Routed to the banner or a toast by call().
    } finally {
      switching = false;
      open = false;
    }
  }
</script>

<Dialog
  bind:open
  title="Switch profile"
  description={target ? `Switch to ${target.name}. Review what changes first.` : "Review what changes first."}
>
  {#if failed}
    <p class="muted">The preview could not be loaded.</p>
  {:else if !preview}
    <p class="muted">Loading preview...</p>
  {:else}
    <div class="sections">
      {#if warning}
        <p class="warning" role="alert">{warning}</p>
      {/if}
      {#if unchanged}
        <p class="muted">The enabled mods are the same in both profiles.</p>
      {/if}
      {#if preview.added_mods.length > 0}
        <section aria-label="Mods added">
          <StencilHeading level={3}>Added ({preview.added_mods.length})</StencilHeading>
          <ul>
            {#each preview.added_mods as id (id)}
              <li>{modName(id)}</li>
            {/each}
          </ul>
        </section>
      {/if}
      {#if preview.removed_mods.length > 0}
        <section aria-label="Mods removed">
          <StencilHeading level={3}>Removed ({preview.removed_mods.length})</StencilHeading>
          <ul>
            {#each preview.removed_mods as id (id)}
              <li>{modName(id)}</li>
            {/each}
          </ul>
        </section>
      {/if}
    </div>
  {/if}
  {#snippet footer()}
    <Button variant="ghost" onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" disabled={!preview || !app.startupDone || switching} onclick={confirm}>Switch profile</Button>
  {/snippet}
</Dialog>

<style>
  .sections {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-4);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  ul {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    margin: 0;
    padding-left: var(--qm-space-4);
    font-size: var(--qm-text-13);
  }

  .muted {
    margin: 0;
    color: var(--qm-text-muted);
    font-size: var(--qm-text-13);
  }

  .warning {
    margin: 0;
    padding: var(--qm-space-3);
    border: 1px solid var(--qm-warn);
    border-radius: var(--qm-radius-sm);
    color: var(--qm-warn);
    font-size: var(--qm-text-13);
  }
</style>
