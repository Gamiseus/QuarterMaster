<script lang="ts">
  import { call } from "../../lib/api";
  import { commands, type ModpackPreview } from "../../lib/bindings";
  import { Badge, Button, Dialog, Select, Skeleton, TextField } from "../../components/ui";
  import { app } from "../../lib/stores/app.svelte";
  import { installs } from "../../lib/stores/installs.svelte";
  import { jobs } from "../../lib/stores/jobs.svelte";
  import { saves } from "../../lib/stores/saves.svelte";

  interface Props {
    open?: boolean;
    /** The `.qmpack` to preview (picked in a file dialog or dropped). */
    path: string | null;
  }

  let { open = $bindable(false), path }: Props = $props();

  const NEW_SLOT = "__new_slot__";

  let preview = $state<ModpackPreview | null>(null);
  let error = $state<string | null>(null);
  let name = $state("");
  let slot = $state(NEW_SLOT);
  let busy = $state(false);
  let seq = 0;

  // A fresh slot by default; the Vanilla slot belongs to the Vanilla profile and is never offered.
  const slotItems = $derived([
    { value: NEW_SLOT, label: "New slot on first launch" },
    ...(saves.list ?? []).filter((s) => !s.profile.builtin && s.profile.id !== "vanilla").map((s) => ({ value: s.profile.id, label: s.profile.name })),
  ]);
  const blocked = $derived(installs.blockReason());
  const missing = $derived(preview?.mods.filter((m) => !m.installed && !m.bundled) ?? []);

  $effect(() => {
    if (!open || !path) return;
    const p = path;
    const mine = ++seq;
    preview = null;
    error = null;
    slot = NEW_SLOT;
    // Shown in the dialog, not as a toast.
    call(commands.modpackInspect(p), { expect: ["invalid_input", "not_found", "io"] })
      .then((r) => {
        if (mine !== seq) return;
        preview = r;
        name = r.name;
      })
      .catch((e) => {
        if (mine === seq) error = e instanceof Error ? e.message : String(e);
      });
  });

  async function importPack() {
    if (!path || !preview || busy) return;
    busy = true;
    const label = name.trim();
    try {
      const id = await call(commands.modpackImport(path, label, slot === NEW_SLOT ? null : slot));
      open = false;
      await jobs.track(id, { label: `Importing ${label}`, success: () => `Imported the modpack "${label}".` });
    } catch {
      // call() or jobs.track already reported it.
    } finally {
      busy = false;
    }
  }
</script>

<Dialog bind:open title="Import modpack" size="lg" dismissible={!busy}>
  {#if error}
    <p role="alert" class="error">{error}</p>
  {:else if !preview}
    <div aria-busy="true" aria-label="Reading the modpack"><Skeleton lines={5} /></div>
  {:else}
    <div class="form">
      {#if preview.description}<p class="qm-muted desc">{preview.description}</p>{/if}
      <TextField label="Profile name" bind:value={name} />
      <Select label="Save slot" items={slotItems} bind:value={slot} />
    </div>
    <ul class="mods" aria-label="Mods in this modpack">
      {#each preview.mods as m (m.id)}
        <li>
          <span class="name">{m.name}</span>
          <span class="qm-muted version">{m.version}</span>
          {#if m.installed}
            <Badge tone="ok">Installed</Badge>
          {:else if m.bundled}
            <Badge tone="info">In the pack</Badge>
          {:else}
            <Badge tone="warn">Missing</Badge>
          {/if}
        </li>
      {/each}
    </ul>
    {#if missing.length > 0}
      <p class="qm-muted note">
        {missing.length === 1 ? "One mod is" : `${missing.length} mods are`} not in the pack or installed. The profile lists
        {missing.length === 1 ? "it" : "them"}; install {missing.length === 1 ? "it" : "them"} from Browse.
      </p>
    {/if}
  {/if}
  {#snippet footer()}
    <Button variant="ghost" disabled={busy} onclick={() => (open = false)}>Cancel</Button>
    <Button
      variant="primary"
      disabled={!preview || !name.trim() || busy || blocked !== null || !app.startupDone}
      title={blocked ?? undefined}
      onclick={importPack}
    >
      Import
    </Button>
  {/snippet}
</Dialog>

<style>
  .form {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--qm-space-3);
    margin-bottom: var(--qm-space-4);
  }

  .desc {
    grid-column: 1 / -1;
    margin: 0;
  }

  .mods {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--qm-text-13);
  }

  .mods li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 8ch auto;
    gap: var(--qm-space-3);
    align-items: center;
    padding: var(--qm-space-2) 0;
    border-bottom: 1px solid var(--qm-border);
  }

  .mods li:last-child {
    border-bottom: 0;
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .version {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  .note,
  .error {
    margin: var(--qm-space-3) 0 0;
    font-size: var(--qm-text-13);
  }
</style>
