<script lang="ts">
  import { untrack } from "svelte";
  import { Badge, Button, Callout, Dialog, EmptyState, Select, Switch } from "../../../components/ui";
  import type { SpriteItem, SpritePlace } from "../../../lib/bindings";
  import { centredOn, fmt, normalTurn, sizedAround, wholeBox, ZOOMS } from "../../../lib/engine/sprites";
  import { sprites, type Ground } from "../../../lib/stores/sprites.svelte";
  import LayerCard from "./LayerCard.svelte";
  import SpriteCanvas from "./SpriteCanvas.svelte";
  import SpritePicker from "./SpritePicker.svelte";

  interface Props {
    folder: string;
  }

  let { folder }: Props = $props();

  let canvas = $state<SpriteCanvas | null>(null);
  let zoom = $state(1);
  let picking = $state<"mine" | "other" | null>(null);
  let mineSet = $state<SpriteItem[]>([]);
  let otherSet = $state<SpriteItem[]>([]);
  /** Set while the "save first?" question waits. */
  let asking = $state<{ resolve: (go: boolean) => void } | null>(null);

  $effect(() => {
    const f = folder;
    sprites.load(f).catch(() => {});
  });

  // An image opened from the Files view.
  $effect(() => {
    const path = sprites.request;
    if (!path || sprites.folder !== folder || sprites.shelves.length === 0) return;
    untrack(() => {
      sprites.request = null;
      void pickMine(path);
    });
  });

  $effect(() => {
    const layer = sprites.mine;
    void sprites.catalogs;
    sprites.setFor(layer).then((s) => (mineSet = s));
  });
  $effect(() => {
    const layer = sprites.other;
    void sprites.catalogs;
    sprites.setFor(layer).then((s) => (otherSet = s));
  });

  const use = $derived(sprites.use);
  const place = $derived(sprites.place);
  const box = $derived(sprites.box);
  const projectName = $derived(sprites.shelves.find((s) => s.source === "workspace" && s.owner === folder)?.label ?? folder);

  /** Unsaved changes to the place are settled before another image or use replaces them. */
  function settle(): Promise<boolean> {
    if (!sprites.dirty) return Promise.resolve(true);
    asking?.resolve(false);
    return new Promise((resolve) => {
      asking = {
        resolve: (go) => {
          asking = null;
          resolve(go);
        },
      };
    });
  }

  async function pickMine(path: string) {
    if (!(await settle())) return;
    await sprites.pickMine(path).catch(() => {});
  }

  async function stepMine(item: SpriteItem) {
    await pickMine(item.sprite.path);
  }

  async function chooseUse(i: number) {
    if (i === sprites.useIndex || !(await settle())) return;
    await sprites.chooseUse(i);
  }

  function fromLabel(item: SpriteItem): string {
    const s = item.sprite;
    if (s.source === "game") return "the game";
    if (s.source === "workspace" && s.owner === folder) return "this mod";
    return sprites.shelves.find((x) => x.source === s.source && x.owner === s.owner)?.label ?? s.owner;
  }

  // ---- The place, from the numbers and buttons ----

  function set(p: SpritePlace) {
    sprites.setPlace(p);
  }

  function num(v: string): number | null {
    const n = Number(v);
    return v.trim() !== "" && Number.isFinite(n) ? n : null;
  }

  function setX(v: string) {
    const n = num(v);
    if (n !== null && place) set({ ...place, x: n });
  }
  function setY(v: string) {
    const n = num(v);
    if (n !== null && place) set({ ...place, y: n });
  }
  function setW(v: string) {
    const n = num(v);
    if (n === null || n <= 0 || !place) return;
    const h = sprites.keepShape ? (n * place.height) / place.width : place.height;
    set(sizedAround(place, { w: n, h: Math.max(0.1, Math.round(h * 100) / 100) }));
  }
  function setH(v: string) {
    const n = num(v);
    if (n === null || n <= 0 || !place) return;
    const w = sprites.keepShape ? (n * place.width) / place.height : place.width;
    set(sizedAround(place, { w: Math.max(0.1, Math.round(w * 100) / 100), h: n }));
  }
  function setTurn(v: string) {
    const n = num(v);
    if (n !== null && place) set({ ...place, rotate: normalTurn(n) });
  }

  /** Your image takes the comparison's size and spot. */
  function fitToIt() {
    const r = sprites.otherRect;
    if (place && r) set({ ...place, x: r.x, y: r.y, width: r.w, height: r.h, rotate: 0 });
  }
  /** Your image's middle on the comparison's middle (the box's middle without one). */
  function centreOnIt() {
    const r = sprites.otherRect;
    const at = r ? { x: r.x + r.w / 2, y: r.y + r.h / 2 } : box ? { x: box.w / 2, y: box.h / 2 } : null;
    if (place && at) set(centredOn(place, at));
  }
  /** Your file's own pixels, one to one game pixel, its middle kept. */
  function actualPixels() {
    const s = sprites.mine?.size;
    if (place && s) set(sizedAround(place, s));
  }
  function resetPlace() {
    if (box) set(wholeBox(box));
  }

  function onkeydown(e: KeyboardEvent) {
    const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement;
    if (typing || !(e.ctrlKey || e.metaKey)) return;
    const k = e.key.toLowerCase();
    if (k === "z" && !e.shiftKey) {
      sprites.undo();
      e.preventDefault();
    } else if (k === "y" || (k === "z" && e.shiftKey)) {
      sprites.redo();
      e.preventDefault();
    } else if (k === "s" && sprites.dirty) {
      void sprites.save();
      e.preventDefault();
    }
  }

  const useItems = $derived(
    (sprites.uses?.uses ?? []).map((u, i) => ({
      value: i,
      label: `${u.global}${u.original ? "" : " (new image)"}, in ${u.target === "qm" ? "qm.toml" : "faction.toml"}`,
    })),
  );
  const grounds: { value: Ground; label: string }[] = [
    { value: "checker", label: "Checks" },
    { value: "dark", label: "Dark" },
    { value: "light", label: "Light" },
  ];
  const zoomText = $derived(`${Math.round(zoom * 100)}%`);
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="sprites" {onkeydown}>
  <section class="controls" aria-label="Sprite line-up">
    <h2 class="qm-stencil head">Sprites of {projectName}</h2>
    {#if !sprites.mine}
      <p class="qm-muted intro">
        Line up an image of this mod with the game's image it replaces: move, size and turn yours over it until it sits right, then save that into the mod. The game then draws it there.
      </p>
      <Button variant="primary" onclick={() => (picking = "mine")}>Pick your sprite</Button>
    {:else}
      <LayerCard
        title="Your sprite"
        who="your sprite"
        layer={sprites.mine}
        from="this mod"
        set={mineSet}
        pickLabel="Pick another"
        onpick={() => (picking = "mine")}
        onstep={stepMine}
      />

      <div class="where">
        {#if useItems.length > 1}
          <Select label="Shown in game as" items={useItems} bind:value={() => sprites.useIndex, (v) => v !== undefined && void chooseUse(v)} />
        {:else if use}
          <p class="small">
            Shown in game as <strong class="mono">{use.global}</strong>{use.original ? ", in place of the game's image" : ", a new image"}.
          </p>
        {/if}
        {#if use?.size_rule}
          <p class="qm-muted small">Its size rule, <span class="mono">{use.size_rule}</span>, sets the box the game's code sees.</p>
        {/if}
        {#if sprites.uses?.note}
          <Callout tone="info">{sprites.uses.note}</Callout>
        {/if}
      </div>

      <LayerCard
        title="Compare with"
        who="the comparison"
        layer={sprites.other}
        from={sprites.other ? fromLabel(sprites.other.item) : ""}
        set={otherSet}
        pickLabel={sprites.other ? "Pick another" : "Pick an image"}
        onpick={() => (picking = "other")}
        onstep={(item) => void sprites.pickOther(item)}
      >
        {#snippet extra()}
          <Switch label="On top" bind:checked={sprites.otherOnTop} />
        {/snippet}
      </LayerCard>

      {#if place}
        <section class="card" aria-label="Place">
          <h3 class="title">Place</h3>
          <div class="nums">
            <label><span>X</span><input type="number" step="1" value={fmt(place.x)} onchange={(e) => setX(e.currentTarget.value)} aria-label="Move right" /></label>
            <label><span>Y</span><input type="number" step="1" value={fmt(place.y)} onchange={(e) => setY(e.currentTarget.value)} aria-label="Move down" /></label>
            <label><span>W</span><input type="number" step="1" min="0.1" value={fmt(place.width)} onchange={(e) => setW(e.currentTarget.value)} aria-label="Width" /></label>
            <label><span>H</span><input type="number" step="1" min="0.1" value={fmt(place.height)} onchange={(e) => setH(e.currentTarget.value)} aria-label="Height" /></label>
            <label><span>Turn</span><input type="number" step="1" value={fmt(place.rotate)} onchange={(e) => setTurn(e.currentTarget.value)} aria-label="Turn in degrees" /></label>
          </div>
          <div class="row">
            <Switch label="Keep shape" bind:checked={sprites.keepShape} />
          </div>
          <div class="row wrap" role="group" aria-label="Mirror">
            <span class="small">Mirror</span>
            <button type="button" class="toggle" aria-pressed={place.flip_x} onclick={() => set({ ...place, flip_x: !place.flip_x })}>Left-right</button>
            <button type="button" class="toggle" aria-pressed={place.flip_y} onclick={() => set({ ...place, flip_y: !place.flip_y })}>Up-down</button>
          </div>
          <div class="buttons">
            <Button size="sm" disabled={!sprites.otherRect} title="Your sprite takes the size and spot of the image you compare with." onclick={fitToIt}>Fit to it</Button>
            <Button size="sm" title="Your sprite's middle on the middle of the image you compare with." onclick={centreOnIt}>Centre on it</Button>
            <Button size="sm" disabled={!sprites.mine?.size} title="Your file's own size: one pixel of the file to one pixel of the game." onclick={actualPixels}>Actual pixels</Button>
            <Button size="sm" title="Fill the game's box, unturned: how the game shows your sprite without a place." onclick={resetPlace}>Reset</Button>
          </div>
          <div class="row">
            <Button size="sm" variant="ghost" disabled={sprites.undoSize === 0} onclick={() => sprites.undo()}>Undo</Button>
            <Button size="sm" variant="ghost" disabled={sprites.redoSize === 0} onclick={() => sprites.redo()}>Redo</Button>
          </div>
        </section>
      {/if}

      <section class="save" aria-label="Save">
        <div class="row wrap">
          <Button variant="primary" disabled={!use || !sprites.dirty || sprites.saving} aria-busy={sprites.saving} onclick={() => void sprites.save()}>
            {sprites.saving ? "Saving..." : "Save to mod"}
          </Button>
          {#if sprites.dirty}<Badge tone="warn">Not saved</Badge>{/if}
        </div>
        {#if use}
          <p class="qm-muted small">Saves into {use.target === "qm" ? "qm.toml" : "faction.toml"}; only what differs from the game's box is written.</p>
        {:else}
          <p class="qm-muted small">The game does not load this file, so its place cannot be saved.</p>
        {/if}
        {#if sprites.lastSaved}
          <Callout tone="info" title="Saved">
            <p class="saved">{sprites.lastSaved.message}</p>
            {#if sprites.lastSaved.written}<pre class="written">{sprites.lastSaved.written}</pre>{/if}
          </Callout>
        {/if}
      </section>
    {/if}
  </section>

  <section class="view" aria-label="Preview">
    <div class="bar">
      <div class="row" role="group" aria-label="Zoom">
        <Button size="sm" variant="ghost" aria-label="Zoom out" disabled={zoom <= ZOOMS[0]} onclick={() => canvas?.zoomBy(-1)}>-</Button>
        <span class="zoom small" aria-live="polite">{zoomText}</span>
        <Button size="sm" variant="ghost" aria-label="Zoom in" disabled={zoom >= ZOOMS[ZOOMS.length - 1]} onclick={() => canvas?.zoomBy(1)}>+</Button>
        <Button size="sm" variant="ghost" onclick={() => canvas?.fit()}>Fit</Button>
        <Button size="sm" variant="ghost" onclick={() => canvas?.zoomTo(1)}>Game size</Button>
      </div>
      <div class="seg" role="group" aria-label="Background">
        {#each grounds as g (g.value)}
          <button type="button" aria-pressed={sprites.ground === g.value} onclick={() => (sprites.ground = g.value)}>{g.label}</button>
        {/each}
      </div>
      <button type="button" class="toggle" aria-pressed={sprites.blinking} disabled={!sprites.mine || !sprites.other} title="Shows one image, then the other, to spot what differs." onclick={() => (sprites.blinking = !sprites.blinking)}>Blink</button>
    </div>
    {#if sprites.mine}
      <SpriteCanvas bind:this={canvas} bind:zoom />
      <p id="sprite-canvas-help" class="qm-muted small help">
        Drag your sprite to move it, a corner to size it (Shift changes its shape), the round handle to turn it (Shift: steps of 15). Arrow keys move it one pixel, Shift ten. Scroll to zoom; drag the background to look around.
      </p>
    {:else}
      <div class="empty">
        <EmptyState title="No sprite yet" body="Pick your sprite on the left. The game's image it replaces is put under it to compare." />
      </div>
      <p id="sprite-canvas-help" hidden></p>
    {/if}
  </section>
</div>

<SpritePicker
  bind:open={() => picking !== null, (v) => {
    if (!v) picking = null;
  }}
  mode={picking ?? "mine"}
  {folder}
  onpick={(item) => (picking === "other" ? void sprites.pickOther(item) : void pickMine(item.sprite.path))}
/>

<Dialog
  bind:open={() => asking !== null, (v) => !v && asking?.resolve(false)}
  title="Save the place first?"
  description={`The place of ${sprites.mine?.item.name ?? "your sprite"} is not saved.`}
>
  {#snippet footer()}
    <Button variant="ghost" onclick={() => asking?.resolve(false)}>Cancel</Button>
    <Button
      onclick={() => {
        sprites.discard();
        asking?.resolve(true);
      }}>Don't save</Button
    >
    <Button variant="primary" data-autofocus="" onclick={async () => asking?.resolve(await sprites.save())}>Save</Button>
  {/snippet}
</Dialog>

<style>
  .sprites {
    display: grid;
    flex: 1;
    grid-template-columns: minmax(14rem, 17rem) minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr);
    gap: var(--qm-space-4);
    min-height: 0;
  }

  .controls {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
    min-height: 0;
    padding-right: var(--qm-space-2);
    overflow-y: auto;
  }

  .head {
    margin: 0;
    font-size: var(--qm-text-16);
  }

  .intro {
    margin: 0;
    font-size: var(--qm-text-13);
  }

  .where {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .where p {
    margin: 0;
  }

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

  .nums {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--qm-space-1) var(--qm-space-2);
  }

  .nums label {
    display: flex;
    align-items: center;
    gap: var(--qm-space-1);
    min-width: 0;
    font-size: var(--qm-text-12);
  }

  .nums label span {
    flex: none;
    width: 2.2rem;
    color: var(--qm-text-muted);
  }

  .nums input {
    flex: 1;
    width: 100%;
    min-width: 0;
    padding: 2px var(--qm-space-1);
    border: 1px solid var(--qm-border-strong);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-surface);
    color: var(--qm-text);
    font: inherit;
    font-variant-numeric: tabular-nums;
  }

  .nums input:focus-visible {
    box-shadow: var(--qm-focus-ring);
    outline: none;
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .wrap {
    flex-wrap: wrap;
  }

  .buttons {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--qm-space-1);
  }

  .small {
    font-size: var(--qm-text-12);
  }

  .mono {
    font-family: var(--qm-font-mono);
  }

  .toggle {
    padding: 2px var(--qm-space-2);
    border: 1px solid var(--qm-border-strong);
    border-radius: var(--qm-radius-sm);
    background: none;
    color: var(--qm-text-muted);
    font: inherit;
    font-size: var(--qm-text-12);
    cursor: pointer;
  }

  .toggle[aria-pressed="true"] {
    background: color-mix(in srgb, var(--qm-accent) 18%, transparent);
    color: var(--qm-text);
  }

  .toggle:disabled {
    cursor: default;
    opacity: 0.5;
  }

  .save {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .save p {
    margin: 0;
  }

  .saved {
    margin: 0;
  }

  .written {
    margin: var(--qm-space-2) 0 0;
    padding: var(--qm-space-2);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-surface-raised);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .view {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    min-width: 0;
    min-height: 0;
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-3);
  }

  .zoom {
    min-width: 3.2rem;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }

  .seg {
    display: inline-flex;
    border: 1px solid var(--qm-border-strong);
    border-radius: var(--qm-radius-sm);
    overflow: hidden;
  }

  .seg button {
    padding: 2px var(--qm-space-2);
    border: 0;
    background: none;
    color: var(--qm-text-muted);
    font: inherit;
    font-size: var(--qm-text-12);
    cursor: pointer;
  }

  .seg button[aria-pressed="true"] {
    background: color-mix(in srgb, var(--qm-accent) 18%, transparent);
    color: var(--qm-text);
  }

  .help {
    flex: none;
    margin: 0;
  }

  .empty {
    display: flex;
    flex: 1;
    align-items: flex-start;
    justify-content: center;
    padding-top: var(--qm-space-6);
    border: 1px dashed var(--qm-border);
    border-radius: var(--qm-radius-sm);
  }
</style>
