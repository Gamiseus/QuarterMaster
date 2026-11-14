<script lang="ts">
  import { ConfirmDialog, Inset } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type ModSummary } from "../../../lib/bindings";
  import { loadOrder } from "../../../lib/stores/loadorder.svelte";
  import { mods } from "../../../lib/stores/mods.svelte";
  import { patches } from "../../../lib/stores/patches.svelte";
  import { selection } from "../../../lib/stores/selection.svelte";
  import { toasts } from "../../../lib/stores/toasts.svelte";

  interface Props {
    open?: boolean;
    /** The mod to uninstall, captured when the dialog opened. */
    target?: ModSummary | null;
    /** Several mods to uninstall in one go; wins over `target`. */
    targets?: ModSummary[];
  }

  let { open = $bindable(false), target = null, targets }: Props = $props();

  const list = $derived(targets ?? (target ? [target] : []));
  const title = $derived(
    list.length === 0 ? "Uninstall" : list.length === 1 ? `Uninstall ${list[0].name}?` : `Uninstall ${list.length} mods?`,
  );

  /** Drops the mod from the selection and focus first, so the detail panel never reloads a removed mod. */
  function forget(id: string): void {
    if (selection.selected.has(id)) selection.selected = new Set([...selection.selected].filter((k) => k !== id));
    if (selection.focused === id) selection.focused = null;
  }

  async function uninstall(): Promise<void> {
    const todo = [...list];
    if (todo.length === 0) return;
    const removed: ModSummary[] = [];
    try {
      await loadOrder.whileChanging(async () => {
        for (const mod of todo) {
          const wasFocused = selection.focused === mod.id;
          forget(mod.id);
          try {
            await call(commands.modUninstall(mod.id));
          } catch (e) {
            if (wasFocused && selection.focused === null) selection.focused = mod.id;
            throw e;
          }
          removed.push(mod);
        }
      });
    } finally {
      if (removed.length > 0) {
        // The mods invalidation can land before the list reload: stop refetching the removed mods now.
        const gone = new Set(removed.map((m) => m.id));
        if (mods.list.status === "ready") patches.retain(mods.list.data.map((m) => m.id).filter((m) => !gone.has(m)));
        toasts.info(removed.length === 1 ? `Uninstalled ${removed[0].name}.` : `Uninstalled ${removed.length} mods.`);
      }
    }
  }
</script>

<ConfirmDialog
  bind:open
  {title}
  body={list.length > 1
    ? "This removes every installed version of these mods and their settings from all profiles."
    : "This removes every installed version and its settings from all profiles."}
  confirmLabel="Uninstall"
  onconfirm={uninstall}
>
  {#if list.length > 1}
    <Inset>
      <ul class="names" aria-label="Mods to uninstall">
        {#each list as m (m.id)}<li>{m.name}</li>{/each}
      </ul>
    </Inset>
  {/if}
</ConfirmDialog>

<style>
  .names {
    max-height: calc(var(--qm-row-h) * 6);
    margin: 0;
    padding-left: var(--qm-space-4);
    overflow-y: auto;
    font-size: var(--qm-text-13);
  }
</style>
