<script lang="ts">
  import type { Snippet } from "svelte";
  import ScrollRegion from "./ScrollRegion.svelte";
  import StencilHeading from "./StencilHeading.svelte";

  interface Props {
    title: string;
    /** One or two plain sentences under the heading saying what the screen is for. */
    intro?: string;
    /** Buttons next to the heading. */
    actions?: Snippet;
    /**
     * True (default): the body is one scroll region under the heading. False: the body is a plain
     * column that fills the height, for screens that lay out their own scroll regions.
     */
    scroll?: boolean;
    class?: string;
    children?: Snippet;
  }

  let { title, intro, actions, scroll = true, class: className = "", children }: Props = $props();
</script>

<!-- A screen fills the window under the title bar; it never scrolls as a page. -->
<section class={["qm-screen", className]} aria-label={title}>
  <header class="head">
    <StencilHeading level={1}>{title}</StencilHeading>
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
  </header>
  {#if intro}<p class="qm-muted intro">{intro}</p>{/if}
  {#if scroll}
    <ScrollRegion class="qm-screen-body scrolls">{@render children?.()}</ScrollRegion>
  {:else}
    <div class="qm-screen-body">{@render children?.()}</div>
  {/if}
</section>

<style>
  .qm-screen {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }

  .head {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-3);
    padding: var(--qm-space-5) var(--qm-space-5) var(--qm-space-4);
  }

  .actions {
    display: flex;
    gap: var(--qm-space-2);
  }

  /* Sits close under the heading, which has its own bottom padding. */
  .intro {
    flex: none;
    margin: calc(var(--qm-space-2) * -1) 0 var(--qm-space-3);
    padding: 0 var(--qm-space-5);
    font-size: var(--qm-text-13);
  }

  .qm-screen :global(.qm-screen-body) {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--qm-space-4);
    min-height: 0;
    padding: 0 var(--qm-space-5) var(--qm-space-5);
  }

  /* A scrolling body keeps its own height; its content stacks at natural size. */
  .qm-screen :global(.qm-screen-body.scrolls) {
    display: block;
  }

  .qm-screen :global(.qm-screen-body.scrolls > * + *) {
    margin-top: var(--qm-space-4);
  }
</style>
