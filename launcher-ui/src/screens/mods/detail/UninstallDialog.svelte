<script lang="ts">
  import { ConfirmDialog } from "../../../components/ui";
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
    target: ModSummary | null;
  }

  let { open = $bindable(false), target }: Props = $props();

  /** Drops the mod from the selection and focus first, so the detail panel never reloads a removed mod. */
  function forget(id: string): void {
    if (selection.selected.has(id)) selection.selected = new Set([...selection.selected].filter((k) => k !== id));
    if (selection.focused === id) selection.focused = null;
  }

  async function uninstall(): Promise<void> {
    if (!target) return;
    const { id, name } = target;
    const wasFocused = selection.focused === id;
    forget(id);
    try {
      await loadOrder.whileChanging(() => call(commands.modUninstall(id)));
    } catch (e) {
      if (wasFocused && selection.focused === null) selection.focused = id;
      throw e;
    }
    // The mods invalidation can land before the list reload: stop refetching the removed mod now.
    if (mods.list.status === "ready") patches.retain(mods.list.data.map((m) => m.id).filter((m) => m !== id));
    toasts.info(`Uninstalled ${name}.`);
  }
</script>

<ConfirmDialog
  bind:open
  title={target ? `Uninstall ${target.name}?` : "Uninstall"}
  body="This removes every installed version and its settings from all profiles."
  confirmLabel="Uninstall"
  onconfirm={uninstall}
/>
