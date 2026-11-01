<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  interface Props extends HTMLAttributes<HTMLDivElement> {
    title: string;
    body: string;
    action?: Snippet;
  }

  let { title, body, action, class: className = "", ...rest }: Props = $props();
</script>

<div class={["empty", className]} {...rest}>
  <h3 class="title">{title}</h3>
  <p class="body">{body}</p>
  {#if action}
    <div class="action">{@render action()}</div>
  {/if}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--qm-space-2);
    padding: var(--qm-space-6);
    text-align: center;
  }

  .title {
    font-size: var(--qm-text-16);
  }

  .body {
    max-width: 40ch;
    margin: 0;
    color: var(--qm-text-muted);
    font-size: var(--qm-text-13);
  }

  .action {
    margin-top: var(--qm-space-3);
  }
</style>
