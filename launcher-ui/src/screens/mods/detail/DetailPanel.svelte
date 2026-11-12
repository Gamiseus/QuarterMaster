<script lang="ts">
  import { untrack } from "svelte";
  import { IconClose, IconMore } from "../../../components/icons";
  import { Button, Checkbox, EmptyState, IconButton, Menu, Panel, ScrollRegion, Skeleton, StencilHeading, Tabs } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type ConfigValue, type ModDetail, type ModSummary } from "../../../lib/bindings";
  import { onEvent } from "../../../lib/events";
  import { stableJson } from "../../../lib/json";
  import { installs } from "../../../lib/stores/installs.svelte";
  import { patches } from "../../../lib/stores/patches.svelte";
  import { modsView } from "../mods-view.svelte";
  import LoadOrderTab from "./LoadOrderTab.svelte";
  import OptionsTab from "./OptionsTab.svelte";
  import OverviewTab from "./OverviewTab.svelte";
  import PatchesTab from "./PatchesTab.svelte";
  import SecurityTab from "./SecurityTab.svelte";
  import UninstallDialog from "./UninstallDialog.svelte";
  import UpdateAction from "./UpdateAction.svelte";

  interface Props {
    /** The focused mod, with the list's current enabled flag. */
    mod: ModSummary | null;
    canMutate: boolean;
    onenabledchange?: (enabled: boolean) => void;
    /** Shown as a Close button in the header (the narrow-window overlay). */
    onclose?: () => void;
  }

  let { mod, canMutate, onenabledchange, onclose }: Props = $props();

  type DetailState = { status: "loading" } | { status: "ready"; data: ModDetail } | { status: "error"; message: string };

  let detail = $state<DetailState>({ status: "loading" });
  let seq = 0;
  let uninstallOpen = $state(false);
  let uninstallTarget = $state<ModSummary | null>(null);

  const uninstallReason = $derived(installs.blockReason("uninstall") ?? undefined);
  const actions = $derived([
    {
      label: "Uninstall...",
      danger: true,
      disabled: uninstallReason !== undefined,
      disabledReason: uninstallReason,
      onselect: () => {
        uninstallTarget = mod;
        uninstallOpen = true;
      },
    },
  ]);

  const modId = $derived(mod?.id ?? null);
  const tabs = $derived([
    { value: "overview", label: "Overview" },
    ...(mod?.has_config ? [{ value: "options", label: "Options" }] : []),
    { value: "load-order", label: "Load order" },
    { value: "patches", label: "Patches" },
    { value: "security", label: "Security" },
  ]);

  /** `quiet` keeps the current detail on screen until the new one lands (a refresh, not a new mod). */
  async function load(id: string, quiet = false): Promise<void> {
    const mine = ++seq;
    if (!quiet || detail.status !== "ready") detail = { status: "loading" };
    try {
      const data = await call(commands.modDetail(id));
      if (mine !== seq) return;
      if (quiet && detail.status === "ready" && detail.data.summary.id === id) {
        // Update only what changed, so an unchanged config keeps the options form's unsaved edits.
        const prev = detail.data;
        const changed = Object.entries(data).filter(([k, v]) => stableJson(prev[k as keyof ModDetail]) !== stableJson(v));
        Object.assign(prev, Object.fromEntries(changed));
      } else {
        detail = { status: "ready", data };
      }
    } catch (e) {
      if (mine !== seq) return;
      // A failed refresh keeps what is shown; the toast from call() reports it.
      if (quiet && detail.status === "ready" && detail.data.summary.id === id) return;
      detail = { status: "error", message: e instanceof Error ? e.message : String(e) };
    }
  }

  $effect(() => {
    const id = modId;
    if (!id) return;
    untrack(() => {
      void load(id);
      patches.load(id).catch(() => {});
    });
    // The mod set changed (enable state, install, update): the open mod's detail may be stale.
    return onEvent((e) => {
      if (e.type === "invalidated" && e.scope === "mods") void load(id, true);
    });
  });

  $effect(() => {
    if (!tabs.some((t) => t.value === modsView.detailTab)) modsView.detailTab = "overview";
  });

  function configSaved(values: Partial<Record<string, ConfigValue>>) {
    if (detail.status === "ready") detail = { status: "ready", data: { ...detail.data, saved_config: values } };
  }
</script>

{#if !mod}
  <Panel>
    <EmptyState title="Select a mod" body="Pick a mod in the list to see its details." />
  </Panel>
{:else}
  <Panel corners class="detail-panel">
    <header class="head">
      <div class="title">
        <StencilHeading level={2}>{mod.name}</StencilHeading>
        <span class="qm-muted version">{mod.kind === "assets" ? "Assets only" : `Version ${mod.version}`}</span>
      </div>
      <div class="head-actions">
        <Checkbox
          checked={mod.enabled}
          label="Enabled"
          disabled={!canMutate}
          onCheckedChange={(v: boolean) => onenabledchange?.(v)}
        />
        <Menu items={actions} align="end">
          {#snippet trigger({ props })}
            <IconButton icon={IconMore} label="More actions" {...props} />
          {/snippet}
        </Menu>
        {#if onclose}
          <IconButton icon={IconClose} label="Close details" onclick={onclose} />
        {/if}
      </div>
    </header>
    <UpdateAction {mod} />
    {#if detail.status === "loading"}
      <div aria-busy="true" aria-label="Loading mod details">
        <Skeleton lines={6} />
      </div>
    {:else if detail.status === "error"}
      <div class="error" role="alert">
        <p>{detail.message}</p>
        <Button size="sm" onclick={() => load(mod.id)}>Retry</Button>
      </div>
    {:else}
      {@const data = detail.data}
      <Tabs bind:value={modsView.detailTab} {tabs} class="detail-tabs" fill>
        {#snippet content(value)}
          <!-- The header and tabs stay put; only this body scrolls. -->
          <ScrollRegion class="body" label={tabs.find((t) => t.value === value)?.label}>
            {#if value === "overview"}
              <OverviewTab detail={data} />
            {:else if value === "options" && data.config_schema}
              <OptionsTab
                modId={mod.id}
                schema={data.config_schema}
                saved={data.saved_config}
                {canMutate}
                onsaved={configSaved}
              />
            {:else if value === "options"}
              <p class="qm-muted">This mod has no options.</p>
            {:else if value === "load-order"}
              <LoadOrderTab modId={mod.id} enabled={mod.enabled} manifest={data.manifest} />
            {:else if value === "patches"}
              <PatchesTab modId={mod.id} {canMutate} />
            {:else}
              <SecurityTab report={data.security_report} integrity={data.integrity} />
            {/if}
          </ScrollRegion>
        {/snippet}
      </Tabs>
    {/if}
  </Panel>
{/if}

<UninstallDialog bind:open={uninstallOpen} target={uninstallTarget} />

<style>
  /* Fills the detail column; the header stays and the tab body scrolls. */
  :global(.detail-panel) {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--qm-space-3);
    min-height: 0;
  }

  .head {
    flex: none;
  }

  /* Five tabs share the fixed panel width. */
  :global(.detail-tabs .qm-tab) {
    flex: 1 1 auto;
    padding-inline: var(--qm-space-1);
    white-space: nowrap;
  }

  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--qm-space-3);
  }

  .head-actions {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .title {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .version {
    font-size: var(--qm-text-12);
  }

  .error {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--qm-space-2);
    font-size: var(--qm-text-13);
  }

  .error p {
    margin: 0;
  }

  /* Readable line length for readmes and option forms, however wide the column is. */
  :global(.detail-panel .body) {
    font-size: var(--qm-text-13);
  }

  :global(.detail-panel .body > *) {
    max-width: var(--qm-measure);
  }
</style>
