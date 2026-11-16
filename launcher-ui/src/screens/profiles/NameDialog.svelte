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
    /** The field's value each time the dialog opens. */
    initial?: string;
    onsubmit: (name: string) => void | Promise<void>;
  }

  let { open = $bindable(false), title, label, submitLabel, description, disabled = false, initial = "", onsubmit }: Props =
    $props();

  const uid = $props.id();
  const formId = `${uid}-form`;
  let name = $state("");
  let busy = $state(false);
  const valid = $derived(name.trim().length > 0);

  $effect(() => {
    if (open) untrack(() => (name = initial));
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
  <form id={formId} onsubmit={submit}>
    <TextField {label} bind:value={name} />
  </form>
  {#snippet footer()}
    <Button variant="ghost" onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" type="submit" form={formId} disabled={!valid || busy || disabled}>{submitLabel}</Button>
  {/snippet}
</Dialog>
