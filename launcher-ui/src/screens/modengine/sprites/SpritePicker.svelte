<script lang="ts">
  import { untrack } from "svelte";
  import { Button, Dialog, Select, Skeleton, TextField } from "../../../components/ui";
  import type { SpriteItem, SpriteShelf } from "../../../lib/bindings";
  import { numbered } from "../../../lib/engine/sprites";
  import { refKey, shelfKey, sprites } from "../../../lib/stores/sprites.svelte";

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
  /** "Other mods" with every one of them at once. */
  const ALL = "*";

  type Tab = "mod" | "game" | "others";
  /** An image and, from more than one mod, the mod's name. */
  type Shown = { item: SpriteItem; from: string | null };
  let tab = $state<Tab>("mod");
  let otherKey = $state<string>(ALL);
  let query = $state("");
  let limit = $state(PAGE);
  let shown = $state<Shown[] | null>(null);
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

  /** The shelves the tab shows: one, or every other mod. */
  const picked = $derived.by((): SpriteShelf[] => {
    if (tab === "mod") return [{ source: "workspace", owner: folder, label: "This mod" }];
    if (tab === "game") return sprites.shelves.filter((s) => s.source === "game");
    if (otherKey === ALL) return others;
    return others.filter((s) => shelfKey(s.source, s.owner) === otherKey);
  });

  // Each time it opens: this mod first when picking yours; the game when comparing.
  $effect(() => {
    if (!open) return;
    untrack(() => {
      query = "";
      limit = PAGE;
      tab = mode === "other" && hasGame ? "game" : "mod";
    });
  });

  let loadSeq = 0;
  $effect(() => {
    const list = picked;
    if (!open || list.length === 0) {
      shown = null;
      return;
    }
    const seq = ++loadSeq;
    loading = true;
    failed = null;
    // Another tab or mod: the last one's images go at once.
    shown = null;
    const many = list.length > 1;
    // A mod whose images cannot be read is left out of "every mod"; alone it says why.
    Promise.allSettled(list.map((s) => sprites.catalog(s.source, s.owner).then((c) => ({ s, c }))))
      .then((results) => {
        if (seq !== loadSeq) return;
        const ok = results.flatMap((r) => (r.status === "fulfilled" ? [r.value] : []));
        const bad = results.find((r) => r.status === "rejected");
        if (ok.length === 0 && bad && bad.status === "rejected") {
          failed = bad.reason instanceof Error ? bad.reason.message : String(bad.reason);
          shown = null;
          return;
        }
        shown = ok.flatMap(({ s, c }) => c.items.map((item) => ({ item, from: many ? s.label : null })));
      })
      .finally(() => {
        if (seq === loadSeq) loading = false;
      });
  });

  /** When comparing: images of the same name as yours (allied_walk for allied_walk1) first. */
  const yourBase = $derived.by(() => {
    if (mode !== "other" || !sprites.mine) return null;
    // The name the game shows it as (allied_fire for a faction's fire.png), else the file's.
    return numbered(sprites.use?.global ?? sprites.mine.item.name).base.toLowerCase();
  });
  const matches = $derived.by(() => {
    const items = shown ?? [];
    const q = query.trim().toLowerCase();
    const found = q
      ? items.filter((i) => i.item.sprite.path.toLowerCase().includes(q) || (i.from ?? "").toLowerCase().includes(q))
      : items;
    const base = yourBase;
    if (!base) return found;
    const same = (i: Shown) => numbered(i.item.name).base.toLowerCase() === base;
    return [...found.filter(same), ...found.filter((i) => !same(i))];
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

  const otherItems = $derived([
    { value: ALL, label: "Every other mod" },
    ...others.map((s) => ({ value: shelfKey(s.source, s.owner), label: s.source === "library" ? `${s.label} (library)` : s.label })),
  ]);
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
      {:else if loading && !shown}
        <Skeleton lines={4} />
      {:else if matches.length === 0}
        <p class="qm-muted">
          {query
            ? "No image has that in its name."
            : mode === "mine" || tab === "mod"
              ? "This mod has no images yet. Put your PNG files in the mod's folder (images that replace the game's go under assets/sprites/), then pick one here."
              : tab === "others"
                ? "No other mod has images."
                : "No images here."}
        </p>
      {:else}
        <ul class="grid" aria-label="Images">
          {#each matches.slice(0, limit) as { item, from } (refKey(item.sprite))}
            <li>
              <button
                type="button"
                class="tile"
                title={from ? `${from}: ${item.sprite.path}` : item.sprite.path}
                aria-label={item.width > 0 ? `${item.name}, ${item.width} x ${item.height}` : item.name}
                onclick={() => choose(item)}
              >
                <span class="thumb"><img use:thumb={item} alt="" draggable="false" /></span>
                <span class="name">{item.name}</span>
                <span class="size">{item.width > 0 ? `${item.width} x ${item.height}` : item.sprite.path.split(".").pop()?.toUpperCase()}{#if from}, {from}{/if}</span>
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
    overflow: hidden;
    color: var(--qm-text-muted);
    font-size: var(--qm-text-11);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .more {
    display: flex;
    justify-content: center;
    padding: var(--qm-space-3) 0;
  }
</style>
