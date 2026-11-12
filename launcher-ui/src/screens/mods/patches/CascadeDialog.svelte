<script lang="ts">
  import { Button, Dialog } from "../../../components/ui";

  interface Props {
    open: boolean;
    title: string;
    message: string;
    /** The other patches the action also changes. */
    items: string[];
    confirmLabel: string;
    /** Disabling is destructive: Cancel stays the primary choice. */
    danger?: boolean;
    /** A second way to go on, such as enabling only this patch. */
    alternateLabel?: string;
    onconfirm: () => Promise<unknown>;
    onalternate?: () => Promise<unknown>;
  }

  let {
    open = $bindable(),
    title,
    message,
    items,
    confirmLabel,
    danger = false,
    alternateLabel,
    onconfirm,
    onalternate,
  }: Props = $props();

  let busy = $state(false);

  async function run(action: (() => Promise<unknown>) | undefined) {
    if (!action) return;
    busy = true;
    try {
      await action();
      open = false;
    } catch {
      // call() already showed the error; keep the dialog open.
    } finally {
      busy = false;
    }
  }
</script>

<Dialog bind:open {title} dismissible={!busy} initialFocus="first">
  <p>{message}</p>
  <ul class="items">
    {#each items as item (item)}<li>{item}</li>{/each}
  </ul>
  {#snippet footer()}
    {#if danger}
      <Button variant="primary" disabled={busy} onclick={() => (open = false)}>Cancel</Button>
      <Button variant="danger" disabled={busy} onclick={() => run(onconfirm)}>{confirmLabel}</Button>
    {:else}
      <Button variant="ghost" disabled={busy} onclick={() => (open = false)}>Cancel</Button>
      {#if alternateLabel}
        <Button variant="secondary" disabled={busy} onclick={() => run(onalternate)}>{alternateLabel}</Button>
      {/if}
      <Button variant="primary" disabled={busy} onclick={() => run(onconfirm)}>{confirmLabel}</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  p {
    margin: 0 0 var(--qm-space-2);
  }

  .items {
    margin: 0;
    padding-left: var(--qm-space-5);
  }
</style>
