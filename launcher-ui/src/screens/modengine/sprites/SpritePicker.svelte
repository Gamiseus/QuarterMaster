<script lang="ts">
  import { untrack } from "svelte";
  import { Button, Dialog, Select, Skeleton, TextField } from "../../../components/ui";
  import type { SpriteCatalog, SpriteItem, SpriteShelf } from "../../../lib/bindings";
  import { shelfKey, sprites } from "../../../lib/stores/sprites.svelte";

  interface Props {
    open?: boolean;
    /** "mine": a file of this mod; "other": any image to compare with. */
    mode: "mine" | "other";
    folder: string;
    onpick: (item: SpriteItem) => void;
  }

  let { open = $bindable(false), mode, folder, onpick }: Props = $props();

  /** How many images show before "Show more". */
  const PAGE = 240;

  type Tab = "mod" | "game" | "others";
  let tab = $state<Tab>("mod");
  let otherKey = $state<string | null>(null);
  let query = $state("");
  let limit = $state(PAGE);
  let catalog = $state<SpriteCatalog | null>(null);
  let loading = $state(false);
  let failed = $state<string | null>(null);

  const others = $derived(sprites.shelves.filter((s) => !(s.source === "workspace" && s.owner === folder) && s.source !== "game"));
  const hasGame = $derived(sprites.shelves.some((s) => s.source === "game"));
  const tabs = $derived(
    mode === "mine"
      ? []
      : ([
          { value: "mod", label: "This mod" },
          ...(hasGame ? [{ value: "game", label: "The game" }] : []),
          { value: "others", label: "Other mods" },
        ] as { value: Tab; label: string }[]),
  );

  const shelf = $derived.by((): SpriteShelf | null => {
    if (tab === "mod") return { source: "workspace", owner: folder, label: "This mod" };
    if (tab === "game") return sprites.shelves.find((s) => s.source === "game") ?? null;
    return others.find((s) => shelfKey(s.source, s.owner) === otherKey) ?? null;
  });

  // Each time it opens: this mod first when picking yours; the game when comparing.
  $effect(() => {
    if (!open) return;
    untrack(() => {
      query = "";
      limit = PAGE;
      tab = mode === "other" && hasGame ? "game" : "mod";
      if (!otherKey && others.length > 0) otherKey = shelfKey(others[0].source, others[0].owner);
    });
  });

  $effect(() => {
    const s = shelf;
    if (!open || !s) {
      catalog = null;
      return;
    }
    loading = true;
    failed = null;
    sprites
      .catalog(s.source, s.owner)
      .then((c) => {
        if (shelf === s) catalog = c;
      })
      .catch((e) => {
        failed = e instanceof Error ? e.message : String(e);
        catalog = null;
      })
      .finally(() => (loading = false));
  });

  const matches = $derived.by(() => {
    const items = catalog?.items ?? [];
    const q = query.trim().toLowerCase();
    return q ? items.filter((i) => i.sprite.path.toLowerCase().includes(q)) : items;
  });

  $effect(() => {
    void query;
    void tab;
    void otherKey;
    limit = PAGE;
  });

  function choose(item: SpriteItem) {
    onpick(item);
    open = false;
  }

  /** Loads a thumbnail when it scrolls into view. */
  function thumb(node: HTMLImageElement, item: SpriteItem) {
    let done = false;
    const io = new IntersectionObserver((entries) => {
      if (done || !entries.some((e) => e.isIntersecting)) return;
      done = true;
      io.disconnect();
      sprites
        .image(item.sprite)
        .then((url) => (node.src = url))
        .catch(() => node.classList.add("broken"));
    });
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  const otherItems = $derived(others.map((s) => ({ value: shelfKey(s.source, s.owner), label: s.source === "library" ? `${s.label} (library)` : s.label })));
</script>

<Dialog
  bind:open
  size="xl"
  title={mode === "mine" ? "Pick your sprite" : "Pick an image to compare with"}
  description={mode === "mine"
    ? "An image of this mod: the one you are working on."
    : "Any image: the game's, this mod's or another mod's. The image you pick is shown centred on the game's box."}
>
  <div class="picker">
    <div class="bar">
      {#if tabs.length > 0}
        <div class="seg" role="group" aria-label="Images from">
          {#each tabs as t (t.value)}
            <button type="button" aria-pressed={tab === t.value} onclick={() => (tab = t.value)}>{t.label}</button>
          {/each}
        </div>
      {/if}
      {#if tab === "others" && others.length > 0}
        <div class="mod-pick">
          <Select label="Mod" hideLabel items={otherItems} bind:value={otherKey} />
        </div>
      {/if}
      <div class="search">
        <TextField label="Find an image" hideLabel placeholder="Find by name or folder" bind:value={query} />
      </div>
    </div>

    <div class="grid-wrap" aria-busy={loading}>
      {#if tab === "others" && others.length === 0}
        <p class="qm-muted">No other mods are in the workspace or the library.</p>
      {:else if failed}
        <p class="qm-muted">{failed}</p>
      {:else if loading && !catalog}
        <Skeleton lines={4} />
      {:else if matches.length === 0}
        <p class="qm-muted">{query ? "No image has that in its name." : mode === "mine" ? "This mod has no images yet. Put your PNG files in the mod's folder (images that replace the game's go under assets/sprites/), then pick one here." : "No images here."}</p>
      {:else}
        <ul class="grid" aria-label="Images">
          {#each matches.slice(0, limit) as item (item.sprite.path)}
            <li>
              <button type="button" class="tile" title={item.sprite.path} aria-label={item.width > 0 ? `${item.name}, ${item.width} x ${item.height}` : item.name} onclick={() => choose(item)}>
                <span class="thumb"><img use:thumb={item} alt="" draggable="false" /></span>
                <span class="name">{item.name}</span>
                <span class="size">{item.width > 0 ? `${item.width} x ${item.height}` : item.sprite.path.split(".").pop()?.toUpperCase()}</span>
              </button>
            </li>
          {/each}
        </ul>
        {#if matches.length > limit}
          <div class="more">
            <Button size="sm" onclick={() => (limit += PAGE)}>Show more ({matches.length - limit} left)</Button>
          </div>
        {/if}
      {/if}
    </div>
  </div>
</Dialog>

<style>
  .picker {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--qm-space-3);
    min-height: 0;
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .search {
    flex: 1;
    min-width: 12rem;
  }

  .mod-pick {
    min-width: 14rem;
  }

  .seg {
    display: inline-flex;
    border: 1px solid var(--qm-border-strong);
    border-radius: var(--qm-radius-sm);
    overflow: hidden;
  }

  .seg button {
    padding: var(--qm-space-1) var(--qm-space-3);
    border: 0;
    background: none;
    color: var(--qm-text-muted);
    font: inherit;
    font-size: var(--qm-text-13);
    cursor: pointer;
  }

  .seg button[aria-pressed="true"] {
    background: color-mix(in srgb, var(--qm-accent) 18%, transparent);
    color: var(--qm-text);
  }

  .grid-wrap {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(7.5rem, 1fr));
    gap: var(--qm-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .tile {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
    width: 100%;
    padding: var(--qm-space-1);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-surface);
    color: var(--qm-text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .tile:hover {
    border-color: var(--qm-border-strong);
  }

  .tile:focus-visible {
    box-shadow: var(--qm-focus-ring);
    outline: none;
  }

  .thumb {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 5rem;
    border-radius: 2px;
    background-color: #9a9a9a;
    background-image: conic-gradient(#c8c8c8 25%, transparent 0 50%, #c8c8c8 0 75%, transparent 0);
    background-size: 12px 12px;
    overflow: hidden;
  }

  .thumb img {
    max-width: 100%;
    max-height: 100%;
    image-rendering: pixelated;
  }

  .thumb img:not([src]) {
    visibility: hidden;
  }

  .name {
    overflow: hidden;
    font-size: var(--qm-text-12);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .size {
    color: var(--qm-text-muted);
    font-size: var(--qm-text-11);
  }

  .more {
    display: flex;
    justify-content: center;
    padding: var(--qm-space-3) 0;
  }
</style>
