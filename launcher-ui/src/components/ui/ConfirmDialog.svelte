<script lang="ts">
  import type { Snippet } from "svelte";
  import Button from "./Button.svelte";
  import Dialog from "./Dialog.svelte";

  interface Props {
    open?: boolean;
    title: string;
    body: string;
    confirmLabel: string;
    /** Destructive confirm (default): Cancel is the primary, focused choice and confirm uses the danger style. */
    danger?: boolean;
    /** Closes the dialog when it resolves; a rejection keeps it open (the caller's call() reports the error). */
    onconfirm: () => unknown | Promise<unknown>;
    /** Confirm cannot be clicked yet, for example while what it does is still being checked. */
    confirmDisabled?: boolean;
    /** Extra content under the body text, such as the files an action removes. */
    children?: Snippet;
  }

  let { open = $bindable(false), title, body, confirmLabel, danger = true, onconfirm, confirmDisabled = false, children }: Props = $props();

  let busy = $state(false);

  async function confirm() {
    if (busy || confirmDisabled) return;
    busy = true;
    try {
      await onconfirm();
      open = false;
    } catch (e) {
      // Command errors were already routed to a toast or banner by call(); anything else must not vanish.
      if (!(e instanceof Error && e.name === "QmCommandError")) console.error(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog bind:open {title} description={body} initialFocus="content" dismissible={!busy}>
  {#if children}{@render children()}{/if}
  {#snippet footer()}
    <Button
      variant={danger ? "primary" : "secondary"}
      disabled={busy}
      data-autofocus={danger ? "" : undefined}
      onclick={() => (open = false)}
    >
      Cancel
    </Button>
    <Button
      variant={danger ? "danger" : "primary"}
      disabled={busy || confirmDisabled}
      aria-busy={busy}
      data-autofocus={danger ? undefined : ""}
      onclick={confirm}
    >
      {confirmLabel}
    </Button>
  {/snippet}
</Dialog>
