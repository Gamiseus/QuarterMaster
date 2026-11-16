<script lang="ts">
  import type { HTMLAttributes } from "svelte/elements";

  interface Props extends HTMLAttributes<HTMLDivElement> {
    lines?: number;
  }

  let { lines = 3, class: className = "", ...rest }: Props = $props();
</script>

<div class={["skeleton", className]} aria-hidden="true" {...rest}>
  {#each { length: lines } as _, i (i)}
    <span class="line"></span>
  {/each}
</div>

<style>
  .skeleton {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .line {
    display: block;
    height: var(--qm-space-3);
    border-radius: var(--qm-radius-sm);
    background: linear-gradient(
      90deg,
      var(--qm-raised) 25%,
      var(--qm-border) 50%,
      var(--qm-raised) 75%
    );
    background-size: 200% 100%;
    animation: shimmer 1.4s var(--qm-ease) infinite;
  }

  .line:last-child {
    width: 60%;
  }

  @keyframes shimmer {
    from {
      background-position: 200% 0;
    }
    to {
      background-position: -200% 0;
    }
  }
</style>
