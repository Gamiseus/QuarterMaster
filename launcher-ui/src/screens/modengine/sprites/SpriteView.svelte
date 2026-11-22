<script lang="ts">
  import { untrack } from "svelte";
  import { Badge, Button, Callout, Dialog, EmptyState, Select, Switch } from "../../../components/ui";
  import type { SpriteItem, SpritePlace } from "../../../lib/bindings";
  import { centredOn, fmt, normalTurn, sizedAround, wholeBox, ZOOMS } from "../../../lib/engine/sprites";
  import { engine, isDirty } from "../../../lib/stores/engine.svelte";
  import { refKey, sprites, type Ground } from "../../../lib/stores/sprites.svelte";
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
  /** Why Match the drawing could not line it up, until the place changes. */
  let matchNote = $state<string | null>(null);
  let matching = $state(false);
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

  // A note about the last Match the drawing goes when the place changes.
  $effect(() => {
    void sprites.place;
    untrack(() => (matchNote = null));
  });

  const use = $derived(sprites.use);
  const place = $derived(sprites.place);
  const box = $derived(sprites.box);
  const projectName = $derived(sprites.shelves.find((s) => s.source === "workspace" && s.owner === folder)?.label ?? folder);
  /** The comparison is something other than the game's image this use replaces. */
  const notTheGames = $derived(
    !!use?.original && (!sprites.other || refKey(sprites.other.item.sprite) !== refKey(use.original.sprite)),
  );
  const target = $derived(use ? (use.target === "qm" ? "qm.toml" : "faction.toml") : null);
  /** The file Save writes has edits not saved in the Files view: saving now would set the two
   * against each other. */
  const editedThere = $derived(!!target && engine.files.some((f) => f.path.toLowerCase() === target && isDirty(f)));

  /** Saves the place, unless the file it goes into has unsaved edits in Files. */
  async function save(): Promise<boolean> {
    if (editedThere) return false;
    return sprites.save();
  }

  function openTarget() {
    if (!target) return;
    engine.view = "files";
    void engine.open(target).catch(() => {});
  }

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

  /** A typed number, or null (and the box shows the place again) when it is not one. */
  function num(input: HTMLInputElement, now: number): number | null {
    const v = input.value;
    const n = Number(v);
    if (v.trim() === "" || !Number.isFinite(n)) {
      input.value = fmt(now);
      return null;
    }
    return n;
  }

  function setX(input: HTMLInputElement) {
    if (!place) return;
    const n = num(input, place.x);
    if (n !== null) set({ ...place, x: n });
  }
  function setY(input: HTMLInputElement) {
    if (!place) return;
    const n = num(input, place.y);
    if (n !== null) set({ ...place, y: n });
  }
  function setW(input: HTMLInputElement) {
    if (!place) return;
    const n = num(input, place.width);
    if (n === null) return;
    if (n <= 0) {
      input.value = fmt(place.width);
      return;
    }
    const h = sprites.keepShape ? (n * place.height) / place.width : place.height;
    set(sizedAround(place, { w: n, h: Math.max(0.1, Math.round(h * 100) / 100) }));
  }
  function setH(input: HTMLInputElement) {
    if (!place) return;
    const n = num(input, place.height);
    if (n === null) return;
    if (n <= 0) {
      input.value = fmt(place.height);
      return;
    }
    const w = sprites.keepShape ? (n * place.width) / place.height : place.width;
    set(sizedAround(place, { w: Math.max(0.1, Math.round(w * 100) / 100), h: n }));
  }
  function setTurn(input: HTMLInputElement) {
    if (!place) return;
    const n = num(input, place.rotate);
    if (n !== null) set({ ...place, rotate: normalTurn(n) });
  }

  /** Sizes and moves your sprite so its drawn part covers the comparison's. */
  async function matchDrawing() {
    matching = true;
    try {
      const why = await sprites.matchDrawn();
      matchNote = why;
    } finally {
      matching = false;
    }
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
  /** Your file at its own size in the game, its middle kept: its pixels over the scale it is drawn at. */
  function actualSize() {
    const s = sprites.mine?.size;
    const k = sprites.fileScale;
    if (place && s) set(sizedAround(place, { w: Math.round((s.w / k) * 100) / 100, h: Math.round((s.h / k) * 100) / 100 }));
  }
  function resetPlace() {
    if (box) set(wholeBox(box));
  }
  function revert() {
    if (sprites.saved) set({ ...sprites.saved });
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
      void save();
      e.preventDefault();
    }
  }

  const useItems = $derived(
    (sprites.uses?.uses ?? []).map((u, i) => ({
      value: i,
      label: `${u.global}${u.original ? "" : " (new image)"}, in ${u.target === "qm" ? "qm.toml" : "faction.toml"}`,
    })),
  );
  const scaleItems = [
    { value: 1, label: "the game's size" },
    { value: 2, label: "2x the game's size" },
    { value: 3, label: "3x the game's size" },
    { value: 4, label: "4x the game's size" },
  ];
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
    <div class="scroll">
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
        >
          {#snippet extra()}
            <div class="scale">
              <Select label="Drawn at" items={scaleItems} bind:value={sprites.fileScale} />
            </div>
          {/snippet}
        </LayerCard>

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
            {#if notTheGames}
              <Button size="sm" variant="ghost" title="Compare with the game's image your sprite replaces again." onclick={() => void sprites.compareWithGame()}>The game's image</Button>
            {/if}
          {/snippet}
        </LayerCard>

        {#if place}
          <section class="card" aria-label="Place">
            <h3 class="title">Place</h3>
            <Button
              size="sm"
              variant="primary"
              disabled={!sprites.otherRect || matching}
              aria-busy={matching}
              title="Sizes and moves your sprite so its drawn part (what is not see-through) covers the drawn part of the image you compare with. Then fine-tune by hand."
              onclick={matchDrawing}>Match the drawing</Button
            >
            {#if matchNote}<p class="note small" role="status">{matchNote}</p>{/if}
            <div class="nums">
              <label title="How far right your sprite's top-left corner is from the box's, in the game's pixels"><span>X</span><input type="number" step="1" value={fmt(place.x)} onchange={(e) => setX(e.currentTarget)} aria-label="Move right" /></label>
              <label title="How far down your sprite's top-left corner is from the box's, in the game's pixels"><span>Y</span><input type="number" step="1" value={fmt(place.y)} onchange={(e) => setY(e.currentTarget)} aria-label="Move down" /></label>
              <label title="The width your sprite is drawn at, in the game's pixels"><span>W</span><input type="number" step="1" min="0.1" value={fmt(place.width)} onchange={(e) => setW(e.currentTarget)} aria-label="Width" /></label>
              <label title="The height your sprite is drawn at, in the game's pixels"><span>H</span><input type="number" step="1" min="0.1" value={fmt(place.height)} onchange={(e) => setH(e.currentTarget)} aria-label="Height" /></label>
              <label title="Degrees clockwise about its middle; minus turns the other way"><span>Turn</span><input type="number" step="1" value={fmt(place.rotate)} onchange={(e) => setTurn(e.currentTarget)} aria-label="Turn in degrees" /></label>
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
              <Button size="sm" disabled={!sprites.mine?.size} title="Your file at its own size in the game: its pixels over the size it is drawn at (above)." onclick={actualSize}>Actual size</Button>
              <Button size="sm" title="Fill the game's box, unturned: how the game shows your sprite without a place." onclick={resetPlace}>Reset</Button>
            </div>
            <div class="row">
              <Button size="sm" variant="ghost" disabled={sprites.undoSize === 0} onclick={() => sprites.undo()}>Undo</Button>
              <Button size="sm" variant="ghost" disabled={sprites.redoSize === 0} onclick={() => sprites.redo()}>Redo</Button>
            </div>
          </section>
        {/if}
      {/if}
    </div>

    {#if sprites.mine}
      <!-- Always in view, however long the column. -->
      <section class="savebar" aria-label="Save">
        <div class="row wrap">
          <Button variant="primary" disabled={!use || !sprites.dirty || sprites.saving || editedThere} aria-busy={sprites.saving} onclick={() => void save()}>
            {sprites.saving ? "Saving..." : "Save to mod"}
          </Button>
          {#if sprites.dirty}
            <Button variant="ghost" title="Back to the place saved in the mod." onclick={revert}>Revert</Button>
            <Badge tone="warn">Not saved</Badge>
          {/if}
        </div>
        {#if editedThere}
          <p class="note small" role="status">
            {target} has changes not saved in the Files view. Save or undo them there first, so neither overwrites the other.
            <button type="button" class="link" onclick={openTarget}>Open it</button>
          </p>
        {:else if sprites.lastSaved}
          <p class="saved small" role="status">{sprites.lastSaved.message}</p>
          {#if sprites.lastSaved.written}
            <details class="lines">
              <summary class="small">The lines in {sprites.lastSaved.file}</summary>
              <pre class="written">{sprites.lastSaved.written}</pre>
            </details>
          {/if}
        {:else if use}
          <p class="qm-muted small">Into {target}, only what differs from the game's box.</p>
        {:else}
          <p class="qm-muted small">The game does not load this file, so its place cannot be saved.</p>
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
      {#if box && sprites.mine}
        <span class="legend small" title="The space the game gives the image. Its code sees this size; your sprite is placed inside it.">
          <span class="swatch box-swatch" aria-hidden="true"></span>Game's box {fmt(box.w)} x {fmt(box.h)}
          <span class="swatch mine-swatch" aria-hidden="true"></span>Your sprite
        </span>
      {/if}
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
    <Button variant="primary" data-autofocus="" disabled={editedThere} onclick={async () => asking?.resolve(await save())}>Save</Button>
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
    min-height: 0;
  }

  .scroll {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--qm-space-3);
    min-height: 0;
    padding-right: var(--qm-space-2);
    padding-bottom: var(--qm-space-3);
    overflow-y: auto;
  }

  .savebar {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: var(--qm-space-1);
    padding-top: var(--qm-space-3);
    border-top: 1px solid var(--qm-border);
  }

  .savebar p {
    margin: 0;
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

  .scale {
    flex: 1;
    min-width: 0;
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

  .note {
    margin: 0;
    color: var(--qm-text-muted);
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

  .saved {
    margin: 0;
    color: var(--qm-ok);
  }

  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--qm-accent-text);
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }

  .lines summary {
    color: var(--qm-text-muted);
    cursor: pointer;
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

  .legend {
    display: inline-flex;
    align-items: center;
    gap: var(--qm-space-1);
    margin-left: auto;
    color: var(--qm-text-muted);
    font-variant-numeric: tabular-nums;
  }

  .swatch {
    display: inline-block;
    width: 14px;
    height: 0;
    margin-left: var(--qm-space-2);
    border-top: 2px dashed #e8c25a;
  }

  .mine-swatch {
    border-top: 2px solid #5fb4ff;
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
