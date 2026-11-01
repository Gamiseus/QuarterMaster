<script lang="ts">
  import { untrack } from "svelte";
  import { Button, Dialog, TextField } from "../../components/ui";

  interface Props {
    open?: boolean;
    title: string;
    label: string;
    submitLabel: string;
    description?: string;
    disabled?: boolean;
    onsubmit: (name: string) => void | Promise<void>;
  }

  let { open = $bindable(false), title, label, submitLabel, description, disabled = false, onsubmit }: Props = $props();

  let name = $state("");
  let busy = $state(false);
  const valid = $derived(name.trim().length > 0);

  $effect(() => {
    if (open) untrack(() => (name = ""));
  });

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!valid || busy || disabled) return;
    busy = true;
    try {
      await onsubmit(name.trim());
      open = false;
    } catch {
      // Routed to a toast by call(); keep the dialog open so the name can be fixed.
    } finally {
      busy = false;
    }
  }
</script>

<Dialog bind:open {title} {description}>
  <form id="name-form" onsubmit={submit}>
    <TextField {label} bind:value={name} />
  </form>
  {#snippet footer()}
    <Button variant="ghost" onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" type="submit" form="name-form" disabled={!valid || busy || disabled}>{submitLabel}</Button>
  {/snippet}
</Dialog>
