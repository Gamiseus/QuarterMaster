<script lang="ts">
  import { untrack } from "svelte";
  import { call } from "../../lib/api";
  import { commands } from "../../lib/bindings";
  import { Button, Dialog, TextField } from "../../components/ui";
  import { app } from "../../lib/stores/app.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";
  import { slug } from "../../lib/slug";

  interface Props {
    open?: boolean;
    mode: "import" | "export";
    /** Export mode: the code to show. */
    code?: string;
  }

  let { open = $bindable(false), mode, code = "" }: Props = $props();

  let input = $state("");
  let name = $state("");
  let busy = $state(false);

  const newId = $derived(slug(name));
  const valid = $derived(input.trim().length > 0 && newId.length > 0);

  $effect(() => {
    if (open) {
      untrack(() => {
        input = "";
        name = "";
      });
    }
  });

  async function copy() {
    try {
      await navigator.clipboard.writeText(code);
      toasts.info("Share code copied.");
    } catch {
      toasts.error("Could not copy the share code.", "Select the code and copy it manually.");
    }
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!valid || busy || !app.startupDone) return;
    busy = true;
    const typed = name.trim();
    let profile;
    try {
      profile = await call(commands.profileImportCode(input.trim(), newId));
    } catch {
      // Routed to a toast by call().
      busy = false;
      return;
    }
    // The import succeeded: close now, then apply the typed name as a separate step.
    open = false;
    try {
      if (profile.name !== typed) await call(commands.profileRename(profile.id, typed));
      toasts.info(`Imported ${typed}. Switch to it from the profile menu.`);
    } catch {
      toasts.info(`Imported as ${profile.name}. Rename it from Manage profiles.`);
    } finally {
      busy = false;
    }
  }
</script>

{#if mode === "export"}
  <Dialog bind:open title="Share code" description="Send this code to another player to share the profile.">
    <TextField label="Share code" value={code} readonly />
    {#snippet footer()}
      <Button variant="ghost" onclick={() => (open = false)}>Close</Button>
      <Button variant="primary" onclick={copy}>Copy share code</Button>
    {/snippet}
  </Dialog>
{:else}
  <Dialog
    bind:open
    title="Import share code"
    description="Create a new profile from a share code. It stays inactive until you switch to it."
  >
    <form id="import-form" class="form" onsubmit={submit}>
      <TextField label="Share code" bind:value={input} placeholder="QM1-..." />
      <TextField label="New profile name" bind:value={name} />
      {#if newId}
        <p class="hint">Profile ID: {newId}</p>
      {/if}
    </form>
    {#snippet footer()}
      <Button variant="ghost" onclick={() => (open = false)}>Cancel</Button>
      <Button variant="primary" type="submit" form="import-form" disabled={!valid || busy || !app.startupDone}>Import</Button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .hint {
    margin: 0;
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }
</style>
