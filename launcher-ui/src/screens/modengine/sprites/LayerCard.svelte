<script lang="ts">
  import type { Snippet } from "svelte";
  import { Button, Checkbox, IconButton } from "../../../components/ui";
  import { IconChevronLeft, IconChevronRight } from "../../../components/icons";
  import type { SpriteItem } from "../../../lib/bindings";
  import { refKey, type Layer } from "../../../lib/stores/sprites.svelte";

  interface Props {
    /** "Your sprite" or "Compare with". */
    title: string;
    /** Names the layer in control labels: "your sprite", "the comparison". */
    who: string;
    layer: Layer | null;
    /** Where the image comes from, in words. */
    from: string;
    /** The numbered set the image is in, to step through. */
    set: SpriteItem[];
    pickLabel: string;
    onpick: () => void;
    onstep: (item: SpriteItem) => void;
    extra?: Snippet;
  }

  let { title, who, layer, from, set, pickLabel, onpick, onstep, extra }: Props = $props();

  const at = $derived(layer ? set.findIndex((i) => refKey(i.sprite) === refKey(layer.item.sprite)) : -1);
  const size = $derived(layer?.size ? `${layer.size.w} x ${layer.size.h}` : "");
</script>

<section class="card" aria-label={title}>
  <h3 class="title">{title}</h3>
  {#if layer}
    <div class="row">
      <span class="thumb">{#if layer.url}<img src={layer.url} alt="" draggable="false" />{/if}</span>
      <div class="meta">
        <span class="name" title={layer.item.sprite.path}>{layer.item.name}</span>
        <span class="qm-muted small">{size}{size && from ? ", " : ""}{from}</span>
        {#if layer.error}<span class="error small">{layer.error}</span>{/if}
      </div>
    </div>
    <div class="row tools">
      <Button size="sm" onclick={onpick}>{pickLabel}</Button>
      {#if set.length > 1 && at >= 0}
        <span class="step" role="group" aria-label={`Other images in the set of ${who}`}>
          <IconButton icon={IconChevronLeft} label="Previous in the set" disabled={at <= 0} onclick={() => onstep(set[at - 1])} />
          <span class="qm-muted small">{at + 1} of {set.length}</span>
          <IconButton icon={IconChevronRight} label="Next in the set" disabled={at >= set.length - 1} onclick={() => onstep(set[at + 1])} />
        </span>
      {/if}
    </div>
    <label class="row opacity">
      <span class="small">See-through</span>
      <input
        type="range"
        min="0"
        max="100"
        step="5"
        aria-label={`Opacity of ${who}`}
        bind:value={layer.opacity}
        style:--fill={`${layer.opacity}%`}
      />
      <span class="small value">{layer.opacity}%</span>
    </label>
    <div class="row">
      <Checkbox label="Show" bind:checked={layer.visible} aria-label={`Show ${who}`} />
      {@render extra?.()}
    </div>
  {:else}
    <Button size="sm" onclick={onpick}>{pickLabel}</Button>
  {/if}
</section>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    padding-bottom: var(--qm-space-3);
    border-bottom: 1px solid var(--qm-border);
  }

  .title {
    margin: 0;
    font-family: var(--qm-font-stencil);
    font-size: var(--qm-text-13);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
    min-width: 0;
  }

  .tools {
    flex-wrap: wrap;
  }

  .thumb {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: 3rem;
    height: 3rem;
    border-radius: 2px;
    background-color: #8f8f8f;
    background-image: conic-gradient(#b4b4b4 25%, transparent 0 50%, #b4b4b4 0 75%, transparent 0);
    background-size: 8px 8px;
    overflow: hidden;
  }

  .thumb img {
    max-width: 100%;
    max-height: 100%;
    image-rendering: pixelated;
  }

  .meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .name {
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .small {
    font-size: var(--qm-text-12);
  }

  .error {
    color: var(--qm-danger);
  }

  .step {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }

  .opacity input {
    flex: 1;
    min-width: 0;
    accent-color: var(--qm-accent);
  }

  .value {
    width: 2.6rem;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
