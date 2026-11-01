<script lang="ts">
  import { untrack } from "svelte";
  import { call } from "../../lib/api";
  import { commands, type LaunchMethodChoice, type ModProfile } from "../../lib/bindings";
  import { Badge, Button, Dialog, Select, StencilHeading, Switch, TextField } from "../../components/ui";
  import { app } from "../../lib/stores/app.svelte";
  import { profiles } from "../../lib/stores/profiles.svelte";
  import { saves } from "../../lib/stores/saves.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";
  import ShareCodeDialog from "./ShareCodeDialog.svelte";

  interface Props {
    open?: boolean;
  }

  let { open = $bindable(false) }: Props = $props();

  const DELETE_ACTIVE_REASON = "Switch to another profile before deleting this one.";

  const METHODS: { value: LaunchMethodChoice; label: string }[] = [
    { value: "default", label: "Default" },
    { value: "steam_url", label: "Steam" },
    { value: "direct_exe", label: "Direct" },
    { value: "vanilla", label: "Vanilla" },
  ];

  let selectedId = $state<string | null>(null);
  let doc = $state<ModProfile | null>(null);
  let name = $state("");
  let saveId = $state("");
  let method = $state<LaunchMethodChoice>("default");
  let args = $state("");
  let disableConsole = $state(false);
  let cloning = $state(false);
  let cloneName = $state("");
  let confirmingDelete = $state(false);
  let busy = $state(false);
  let shareCode = $state("");
  let shareOpen = $state(false);
  let loadSeq = 0;

  const selected = $derived(profiles.list.find((p) => p.id === selectedId) ?? null);
  const isActive = $derived(selected?.is_active ?? false);
  const canEdit = $derived(app.startupDone && !busy && doc !== null);
  const saveItems = $derived((saves.list ?? []).map((s) => ({ value: s.profile.id, label: s.profile.name })));

  function parseArgs(text: string): string[] {
    return text.split(/\s+/).filter(Boolean);
  }

  const settingsDirty = $derived(
    doc !== null &&
      (method !== (doc.launch_method ?? "default") ||
        parseArgs(args).join(" ") !== (doc.custom_launch_args ?? []).join(" ") ||
        disableConsole !== (doc.disable_console ?? false) ||
        saveId !== (doc.linked_save_profile_id ?? "")),
  );
  const renameDirty = $derived(doc !== null && name.trim().length > 0 && name.trim() !== doc.name);

  function apply(d: ModProfile) {
    doc = d;
    name = d.name;
    saveId = d.linked_save_profile_id ?? "";
    method = d.launch_method ?? "default";
    args = (d.custom_launch_args ?? []).join(" ");
    disableConsole = d.disable_console ?? false;
  }

  function select(id: string) {
    selectedId = id;
    cloning = false;
    confirmingDelete = false;
  }

  // Start on the active profile each time the dialog opens.
  $effect(() => {
    if (!open) return;
    untrack(() => select(profiles.activeSummary?.id ?? profiles.list[0]?.id ?? ""));
  });

  // Load the selected profile's document.
  $effect(() => {
    const id = selectedId;
    if (!open || !id) return;
    const mine = ++loadSeq;
    doc = null;
    call(commands.profileGet(id))
      .then((d) => {
        if (mine === loadSeq) apply(d);
      })
      .catch(() => {});
  });

  async function run(action: () => Promise<void>) {
    if (busy) return;
    busy = true;
    try {
      await action();
    } catch {
      // Routed to the banner or a toast by call().
    } finally {
      busy = false;
    }
  }

  const rename = () =>
    run(async () => {
      if (!selectedId) return;
      apply(await call(commands.profileRename(selectedId, name.trim())));
      toasts.info("Profile renamed.");
    });

  const saveSettings = () =>
    run(async () => {
      if (!selectedId || !doc) return;
      const relink = saveId !== (doc.linked_save_profile_id ?? "") && saveId !== "";
      apply(
        await call(
          commands.profileUpdate(selectedId, {
            launch_method: method,
            custom_launch_args: parseArgs(args),
            disable_console: disableConsole,
            linked_save_profile_id: relink ? saveId : null,
          }),
        ),
      );
      toasts.info("Profile settings saved.");
    });

  function startClone() {
    cloneName = doc ? `${doc.name} copy` : "";
    cloning = true;
  }

  const clone = () =>
    run(async () => {
      if (!selectedId) return;
      const copy = await call(commands.profileClone(selectedId, cloneName.trim()));
      cloning = false;
      select(copy.id);
      toasts.info("Profile cloned.");
    });

  const remove = () =>
    run(async () => {
      if (!selectedId) return;
      await call(commands.profileDelete(selectedId));
      confirmingDelete = false;
      toasts.info("Profile deleted.");
      select(profiles.activeSummary?.id ?? "");
    });

  const copyCode = () =>
    run(async () => {
      if (!selectedId) return;
      const code = await call(commands.profileExportCode(selectedId));
      try {
        await navigator.clipboard.writeText(code);
        toasts.info("Share code copied.");
      } catch {
        // No clipboard access: show the code so it can be copied by hand.
        shareCode = code;
        shareOpen = true;
      }
    });
</script>

<Dialog bind:open title="Manage profiles">
  <div class="layout">
    <ul class="list" aria-label="Profiles">
      {#each profiles.list as p (p.id)}
        <li>
          <button type="button" class="entry" aria-pressed={p.id === selectedId} onclick={() => select(p.id)}>
            <span class="entry-name">{p.name}</span>
            {#if p.is_active}
              <Badge tone="accent">Active</Badge>
            {/if}
          </button>
        </li>
      {/each}
    </ul>

    {#if selected && doc}
      <section class="detail" aria-label="Profile settings">
        <StencilHeading level={3}>Name</StencilHeading>
        <div class="row">
          <TextField class="grow" label="Profile name" bind:value={name} disabled={!canEdit} />
          <Button disabled={!canEdit || !renameDirty} onclick={rename}>Rename</Button>
        </div>

        <StencilHeading level={3}>Launch</StencilHeading>
        <Select label="Linked save slot" items={saveItems} bind:value={saveId} disabled={!canEdit} placeholder="No save slot" />
        <Select label="Launch method" items={METHODS} bind:value={method} disabled={!canEdit} />
        <TextField label="Launch arguments" placeholder="-windowed -novid" bind:value={args} disabled={!canEdit} />
        <p class="hint">Separate arguments with spaces. Quotes are not supported.</p>
        <Switch label="Disable console" bind:checked={disableConsole} disabled={!canEdit} />
        <div class="row end">
          <Button variant="primary" disabled={!canEdit || !settingsDirty} onclick={saveSettings}>Save settings</Button>
        </div>

        <StencilHeading level={3}>Actions</StencilHeading>
        {#if cloning}
          <div class="row">
            <TextField class="grow" label="Name for the copy" bind:value={cloneName} />
            <Button variant="primary" disabled={!canEdit || cloneName.trim().length === 0} onclick={clone}>Create copy</Button>
            <Button variant="ghost" onclick={() => (cloning = false)}>Cancel</Button>
          </div>
        {:else if confirmingDelete}
          <div class="row">
            <p class="confirm">Delete {selected.name}? This cannot be undone.</p>
            <Button variant="danger" disabled={!canEdit} onclick={remove}>Delete</Button>
            <Button variant="ghost" onclick={() => (confirmingDelete = false)}>Keep</Button>
          </div>
        {:else}
          <div class="row">
            <Button disabled={!canEdit} onclick={startClone}>Clone</Button>
            <Button disabled={!app.startupDone || busy} onclick={copyCode}>Copy share code</Button>
            <Button
              variant="danger"
              disabled={!canEdit || isActive}
              {...{ "aria-description": isActive ? DELETE_ACTIVE_REASON : undefined }}
              onclick={() => (confirmingDelete = true)}
            >
              Delete
            </Button>
          </div>
          {#if isActive}
            <p class="reason">{DELETE_ACTIVE_REASON}</p>
          {/if}
        {/if}
      </section>
    {:else}
      <p class="reason">Loading...</p>
    {/if}
  </div>
</Dialog>

<ShareCodeDialog bind:open={shareOpen} mode="export" code={shareCode} />

<style>
  .layout {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-4);
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .entry {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-2);
    width: 100%;
    min-height: var(--qm-row-h);
    padding: var(--qm-space-1) var(--qm-space-3);
    border: 1px solid transparent;
    border-radius: var(--qm-radius-sm);
    background: transparent;
    color: var(--qm-text);
    font-size: var(--qm-text-13);
    text-align: left;
  }

  .entry:hover {
    background: var(--qm-raised);
  }

  .entry[aria-pressed="true"] {
    border-color: var(--qm-accent);
    background: var(--qm-raised);
  }

  .entry-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .detail {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--qm-space-2);
  }

  .row.end {
    justify-content: flex-end;
  }

  .row :global(.grow) {
    flex: 1;
    min-width: 0;
  }

  .hint {
    margin: calc(var(--qm-space-1) * -1) 0 0;
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }

  .reason,
  .confirm {
    margin: 0;
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }

  .confirm {
    flex: 1;
    align-self: center;
    color: var(--qm-text);
    font-size: var(--qm-text-13);
  }
</style>
