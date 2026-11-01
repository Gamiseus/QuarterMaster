<script lang="ts">
  import { call } from "../../lib/api";
  import { commands } from "../../lib/bindings";
  import { IconChevronDown } from "../../components/icons";
  import { Menu, mergeProps } from "../../components/ui";
  import { app } from "../../lib/stores/app.svelte";
  import { prelaunch } from "../../lib/stores/prelaunch.svelte";
  import { profiles } from "../../lib/stores/profiles.svelte";
  import { saves } from "../../lib/stores/saves.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";
  import ManageProfilesDialog from "./ManageProfilesDialog.svelte";
  import NameDialog from "./NameDialog.svelte";
  import ShareCodeDialog from "./ShareCodeDialog.svelte";
  import SwitchPreviewDialog from "./SwitchPreviewDialog.svelte";

  let previewOpen = $state(false);
  let previewId = $state<string | null>(null);
  let manageOpen = $state(false);
  let importOpen = $state(false);
  let newOpen = $state(false);

  const activeName = $derived(profiles.activeSummary?.name ?? profiles.active?.name ?? null);
  const label = $derived(prelaunch.vanilla ? `Vanilla (${activeName ?? "no profile"})` : (activeName ?? "QuarterMaster"));
  const notReady = "Available once startup has finished.";

  function choose(id: string) {
    const target = profiles.list.find((p) => p.id === id);
    if (!target) return;
    if (target.is_active) {
      prelaunch.vanilla = false;
      return;
    }
    previewId = id;
    previewOpen = true;
  }

  async function createProfile(name: string) {
    const created = await call(commands.profileCreate({ name, id: null, clone_from: null, linked_save_profile_id: null, description: null }));
    toasts.info(`Created ${created.name}.`);
  }

  const items = $derived([
    ...profiles.list.map((p) => ({
      label: p.name,
      description: `${p.enabled_count} ${p.enabled_count === 1 ? "mod" : "mods"}${p.linked_save_profile_id ? `, ${saves.nameFor(p.linked_save_profile_id)}` : ""}`,
      checked: p.is_active && !prelaunch.vanilla,
      disabled: !app.startupDone,
      disabledReason: app.startupDone ? undefined : notReady,
      onselect: () => choose(p.id),
    })),
    {
      label: "Vanilla",
      description: "Launch the game without mods",
      checked: prelaunch.vanilla,
      separatorBefore: true,
      onselect: () => {
        prelaunch.vanilla = true;
      },
    },
    {
      label: "New profile",
      separatorBefore: true,
      disabled: !app.startupDone,
      disabledReason: app.startupDone ? undefined : notReady,
      onselect: () => {
        newOpen = true;
      },
    },
    {
      label: "Manage profiles",
      onselect: () => {
        manageOpen = true;
      },
    },
    {
      label: "Import share code",
      disabled: !app.startupDone,
      disabledReason: app.startupDone ? undefined : notReady,
      onselect: () => {
        importOpen = true;
      },
    },
  ]);
</script>

<Menu {items}>
  {#snippet trigger({ props })}
    <button type="button" class="picker" aria-label="Profile: {label}" {...mergeProps(props, { "data-testid": "profile-picker" })}>
      <span class="name" data-testid="profile-name">{label}</span>
      <IconChevronDown />
    </button>
  {/snippet}
</Menu>

<SwitchPreviewDialog bind:open={previewOpen} profileId={previewId} />
<ManageProfilesDialog bind:open={manageOpen} />
<NameDialog
  bind:open={newOpen}
  title="New profile"
  label="Profile name"
  submitLabel="Create"
  disabled={!app.startupDone}
  onsubmit={createProfile}
/>
<ShareCodeDialog bind:open={importOpen} mode="import" />

<style>
  .picker {
    display: inline-flex;
    align-items: center;
    gap: var(--qm-space-2);
    max-width: var(--qm-detail-w);
    min-height: var(--qm-row-h);
    padding: var(--qm-space-1) var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
    color: var(--qm-text);
    font-size: var(--qm-text-13);
    font-weight: 600;
    transition: border-color var(--qm-dur-fast) var(--qm-ease);
  }

  .picker:hover {
    border-color: var(--qm-accent);
  }

  .picker :global(.qm-icon) {
    flex: none;
    width: var(--qm-text-12);
    height: var(--qm-text-12);
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
