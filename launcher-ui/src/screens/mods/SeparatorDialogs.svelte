<script lang="ts">
  import { Button, Dialog } from "../../components/ui";
  import type { ProfileSeparator } from "../../lib/bindings";
  import { rowKey } from "../../lib/modlist/order";
  import { selection } from "../../lib/stores/selection.svelte";
  import { modList } from "./modlist-controller.svelte";
  import SeparatorDialog from "./SeparatorDialog.svelte";

  interface Props {
    /** The list's keyboard cursor row; a new separator goes before it. */
    cursor: string | null;
  }

  let { cursor }: Props = $props();

  let editOpen = $state(false);
  let editing = $state<ProfileSeparator | null>(null);
  let removeOpen = $state(false);
  let removing = $state<ProfileSeparator | null>(null);

  /** Opens the add dialog (no separator) or the edit dialog for `sep`. */
  export function edit(sep: ProfileSeparator | null) {
    editing = sep;
    editOpen = true;
  }

  export function remove(sep: ProfileSeparator) {
    removing = sep;
    removeOpen = true;
  }

  function submit(title: string, color: string) {
    if (editing) void modList.updateSeparator(editing.id, { title, color });
    else {
      // Before the cursor row, else the focused row; at the end only when that row is gone.
      const key = cursor ?? selection.focused;
      const exists = key !== null && modList.rows.some((r) => rowKey(r) === key);
      void modList.addSeparator(title, color, exists ? key : null);
    }
  }

  function confirmRemove() {
    if (removing) void modList.removeSeparator(removing.id);
    removeOpen = false;
  }
</script>

<SeparatorDialog
  bind:open={editOpen}
  mode={editing ? "edit" : "add"}
  initial={editing ? { title: editing.title, color: editing.color } : { title: "", color: "sep-1" }}
  onsubmit={submit}
/>

<Dialog
  bind:open={removeOpen}
  title="Remove separator"
  description={removing ? `Remove "${removing.title}"? The mods under it stay where they are.` : ""}
>
  {#snippet footer()}
    <Button variant="ghost" onclick={() => (removeOpen = false)}>Cancel</Button>
    <Button variant="danger" onclick={confirmRemove}>Remove</Button>
  {/snippet}
</Dialog>
