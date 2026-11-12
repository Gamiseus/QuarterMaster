<script lang="ts">
  import type { Snippet } from "svelte";
  import StencilHeading from "./StencilHeading.svelte";

  interface Props {
    title: string;
    description?: string;
    /** Buttons on the right of the section heading. */
    actions?: Snippet;
    class?: string;
    children?: Snippet;
  }

  let { title, description, actions, class: className = "", children }: Props = $props();

  const uid = $props.id();
</script>

<!-- A titled group inside a card or screen; sections after the first get a divider above them. -->
<section class={["qm-section", className]} aria-labelledby="{uid}-title">
  <header class="head">
    <div class="titles">
      <StencilHeading level={3} id="{uid}-title">{title}</StencilHeading>
      {#if description}<p class="qm-muted description">{description}</p>{/if}
    </div>
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
  </header>
  <div class="body">{@render children?.()}</div>
</section>

<style>
  .qm-section {
    display: flex;
    flex-direction: column;
    gap: var(--qm-gap-group);
  }

  :global(.qm-section + .qm-section) {
    padding-top: var(--qm-gap-section);
    border-top: 1px solid var(--qm-border);
  }

  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--qm-space-3);
  }

  .titles {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  .description {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .actions {
    display: flex;
    flex: none;
    gap: var(--qm-space-2);
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: var(--qm-gap-item);
  }
</style>
