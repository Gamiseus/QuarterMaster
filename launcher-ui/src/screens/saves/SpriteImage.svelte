<script lang="ts">
  import type { SpriteKind } from "../../lib/bindings";
  import { saves } from "../../lib/stores/saves.svelte";

  interface Props {
    kind: SpriteKind;
    id: number;
    alt: string;
    class?: string;
  }

  let { kind, id, alt, class: className = "" }: Props = $props();

  const url = $derived(saves.sprites.get(`${kind}:${id}`));
  let box = $state<HTMLElement | null>(null);

  // Lazy: ask for the image once its box is near the view (immediately without IntersectionObserver).
  $effect(() => {
    if (!box || url !== undefined) return;
    const k = kind;
    const n = id;
    if (typeof IntersectionObserver === "undefined") {
      saves.loadSprite(k, n);
      return;
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          saves.loadSprite(k, n);
          io.disconnect();
        }
      },
      { rootMargin: "200px" },
    );
    io.observe(box);
    return () => io.disconnect();
  });
</script>

<span class={["sprite", className]} bind:this={box}>
  {#if url}
    <img src={url} {alt} />
  {:else}
    <!-- Placeholder while loading, or when the game has no such image. -->
    <span class="placeholder" role="img" aria-label={alt}></span>
  {/if}
</span>

<style>
  .sprite {
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  img {
    max-width: 100%;
    max-height: 100%;
    image-rendering: pixelated;
  }

  .placeholder {
    display: block;
    width: 100%;
    height: 100%;
    min-width: 16px;
    min-height: 16px;
    border-radius: var(--qm-radius-sm);
    background: var(--qm-raised);
  }
</style>
