<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import type { SpritePlace } from "../../../lib/bindings";
  import { bounds, centreOf, corners, fmt, moved, resized, turnHandle, turned, zoomStep, type Point } from "../../../lib/engine/sprites";
  import { refKey, sprites, type Ground } from "../../../lib/stores/sprites.svelte";

  /** Screen pixels a handle reaches, and the turn handle's distance above the top edge. */
  const HANDLE = 9;
  const TURN_GAP = 26;
  const MARGIN = 40;

  interface Props {
    /** Screen pixels per game pixel. */
    zoom?: number;
  }

  let { zoom = $bindable(1) }: Props = $props();

  let stage = $state<HTMLDivElement | null>(null);
  let width = $state(0);
  let height = $state(0);
  /** Where the box's top-left corner is on the stage, in screen pixels. */
  let origin = $state<Point>({ x: 0, y: 0 });
  let pointer = $state<Point | null>(null);
  let blinkMine = $state(true);
  let fitted = "";
  /** The view was zoomed or moved by hand: it stays as it is. */
  let manual = false;

  type Drag =
    | { kind: "move"; from: Point; start: SpritePlace }
    | { kind: "resize"; corner: number; start: SpritePlace }
    | { kind: "turn"; start: SpritePlace }
    | { kind: "pan"; from: Point; origin: Point };
  let drag: Drag | null = null;
  let cursor = $state("default");

  const place = $derived(sprites.place);
  const box = $derived(sprites.box);
  const otherRect = $derived(sprites.otherRect);
  const mineShown = $derived(!!sprites.mine?.visible && (!sprites.blinking || blinkMine));
  const otherShown = $derived(!!sprites.other?.visible && (!sprites.blinking || !blinkMine));
  const sharp = $derived(zoom > 1);

  const toScreen = (p: Point): Point => ({ x: origin.x + p.x * zoom, y: origin.y + p.y * zoom });
  const toWorld = (p: Point): Point => ({ x: (p.x - origin.x) / zoom, y: (p.y - origin.y) / zoom });

  /** Shows the box, the comparison and your image, at a whole zoom when it fits. It keeps
   * fitting as the canvas changes size until you zoom or look around yourself. */
  export function fit(): void {
    manual = false;
    if (!box || width <= 0 || height <= 0) return;
    const b = bounds(box, otherRect ? [otherRect] : [], place);
    const room = Math.min((width - 2 * MARGIN) / Math.max(1, b.w), (height - 2 * MARGIN) / Math.max(1, b.h));
    const z = room >= 1 ? Math.min(16, Math.floor(room)) : Math.max(0.1, room);
    zoom = z;
    origin = { x: width / 2 - (b.x + b.w / 2) * z, y: height / 2 - (b.y + b.h / 2) * z };
  }

  /** Zooms to `z`, keeping the game pixel under `at` (the stage's middle) where it is. */
  export function zoomTo(z: number, at?: Point): void {
    manual = true;
    const p = at ?? { x: width / 2, y: height / 2 };
    const w = toWorld(p);
    zoom = z;
    origin = { x: p.x - w.x * z, y: p.y - w.y * z };
  }

  export function zoomBy(dir: 1 | -1): void {
    zoomTo(zoomStep(zoom, dir));
  }

  // Fits for a new image, use or comparison, and as the canvas changes size, until the view
  // was zoomed or moved by hand.
  $effect(() => {
    const key = `${sprites.mine ? refKey(sprites.mine.item.sprite) : ""}|${sprites.useIndex}|${box?.w}x${box?.h}|${otherRect ? `${otherRect.w}x${otherRect.h}` : ""}`;
    const w = width;
    const h = height;
    untrack(() => {
      if (key !== fitted) {
        fitted = key;
        manual = false;
      }
      if (!manual && box && w > 0 && h > 0) fit();
    });
  });

  let blinkTimer: ReturnType<typeof setInterval> | null = null;
  $effect(() => {
    if (blinkTimer) clearInterval(blinkTimer);
    blinkTimer = null;
    blinkMine = true;
    if (sprites.blinking) blinkTimer = setInterval(() => (blinkMine = !blinkMine), 600);
  });
  onDestroy(() => {
    if (blinkTimer) clearInterval(blinkTimer);
  });

  function local(e: PointerEvent | WheelEvent): Point {
    const r = stage?.getBoundingClientRect();
    return { x: e.clientX - (r?.left ?? 0), y: e.clientY - (r?.top ?? 0) };
  }

  /** What is under a stage point: a corner, the turn handle, your image, or nothing. */
  function hit(p: Point): { kind: "corner"; i: number } | { kind: "turn" } | { kind: "body" } | null {
    if (!place || !sprites.mine?.visible) return null;
    const near = (q: Point) => Math.hypot(q.x - p.x, q.y - p.y) <= HANDLE;
    const t = toScreen(turnHandle(place, TURN_GAP / zoom));
    if (near(t)) return { kind: "turn" };
    const cs = corners(place).map(toScreen);
    for (let i = 0; i < 4; i++) if (near(cs[i])) return { kind: "corner", i };
    const w = toWorld(p);
    const c = centreOf(place);
    const r = (-place.rotate * Math.PI) / 180;
    const dx = w.x - c.x;
    const dy = w.y - c.y;
    const lx = dx * Math.cos(r) - dy * Math.sin(r);
    const ly = dx * Math.sin(r) + dy * Math.cos(r);
    if (Math.abs(lx) <= place.width / 2 && Math.abs(ly) <= place.height / 2) return { kind: "body" };
    return null;
  }

  const CORNER_CURSORS = ["nwse-resize", "nesw-resize", "nwse-resize", "nesw-resize"];

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0 && e.button !== 1) return;
    stage?.focus();
    const p = local(e);
    const h = e.button === 1 ? null : hit(p);
    if (h && place) {
      sprites.beginChange();
      if (h.kind === "body") drag = { kind: "move", from: toWorld(p), start: { ...place } };
      else if (h.kind === "corner") drag = { kind: "resize", corner: h.i, start: { ...place } };
      else drag = { kind: "turn", start: { ...place } };
    } else {
      manual = true;
      drag = { kind: "pan", from: p, origin: { ...origin } };
      cursor = "grabbing";
    }
    stage?.setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  function onpointermove(e: PointerEvent) {
    const p = local(e);
    pointer = toWorld(p);
    const d = drag;
    if (!d) {
      const h = hit(p);
      cursor = !h ? "grab" : h.kind === "body" ? "move" : h.kind === "turn" ? "crosshair" : CORNER_CURSORS[h.i];
      return;
    }
    const w = toWorld(p);
    if (d.kind === "pan") {
      origin = { x: d.origin.x + p.x - d.from.x, y: d.origin.y + p.y - d.from.y };
    } else if (d.kind === "move") {
      sprites.setPlace(moved(d.start, { x: w.x - d.from.x, y: w.y - d.from.y }, e.altKey), false);
    } else if (d.kind === "resize") {
      const keep = sprites.keepShape !== e.shiftKey;
      sprites.setPlace(resized(d.start, d.corner, w, keep, e.altKey), false);
    } else {
      sprites.setPlace(turned(d.start, w, e.shiftKey ? 15 : 0, e.altKey), false);
    }
  }

  function onpointerup(e: PointerEvent) {
    if (drag?.kind === "pan") cursor = "grab";
    drag = null;
    if (stage?.hasPointerCapture(e.pointerId)) stage.releasePointerCapture(e.pointerId);
  }

  function onwheel(e: WheelEvent) {
    e.preventDefault();
    zoomTo(zoomStep(zoom, e.deltaY < 0 ? 1 : -1), local(e));
  }

  function onkeydown(e: KeyboardEvent) {
    const p = place;
    const step = e.shiftKey ? 10 : 1;
    const arrows: Record<string, Point> = { ArrowLeft: { x: -step, y: 0 }, ArrowRight: { x: step, y: 0 }, ArrowUp: { x: 0, y: -step }, ArrowDown: { x: 0, y: step } };
    const a = arrows[e.key];
    if (a && p && sprites.mine?.visible) {
      sprites.setPlace(moved(p, a));
      e.preventDefault();
    } else if (e.key === "+" || e.key === "=") {
      zoomBy(1);
      e.preventDefault();
    } else if (e.key === "-") {
      zoomBy(-1);
      e.preventDefault();
    } else if (e.key === "0") {
      fit();
      e.preventDefault();
    }
  }

  const groundClass = (g: Ground) => `ground-${g}`;

  // Screen rectangles of the two images.
  const mineStyle = $derived.by(() => {
    if (!place) return "";
    const s = toScreen({ x: place.x, y: place.y });
    const fx = place.flip_x ? -1 : 1;
    const fy = place.flip_y ? -1 : 1;
    return `left:${s.x}px;top:${s.y}px;width:${place.width * zoom}px;height:${place.height * zoom}px;transform:rotate(${place.rotate}deg) scale(${fx},${fy});opacity:${(sprites.mine?.opacity ?? 100) / 100}`;
  });
  const otherStyle = $derived.by(() => {
    const r = otherRect;
    if (!r) return "";
    const s = toScreen({ x: r.x, y: r.y });
    return `left:${s.x}px;top:${s.y}px;width:${r.w * zoom}px;height:${r.h * zoom}px;opacity:${(sprites.other?.opacity ?? 100) / 100}`;
  });

  const boxRect = $derived.by(() => {
    if (!box) return null;
    const s = toScreen({ x: 0, y: 0 });
    return { x: s.x, y: s.y, w: box.w * zoom, h: box.h * zoom };
  });
  const outline = $derived(place ? corners(place).map(toScreen) : []);
  const turnAt = $derived(place ? toScreen(turnHandle(place, TURN_GAP / zoom)) : null);
  const topMid = $derived(outline.length === 4 ? { x: (outline[0].x + outline[1].x) / 2, y: (outline[0].y + outline[1].y) / 2 } : null);
</script>

<!-- The canvas takes the keyboard (arrow keys move your sprite) and the mouse. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class={["stage", groundClass(sprites.ground)]}
  bind:this={stage}
  bind:clientWidth={width}
  bind:clientHeight={height}
  role="application"
  aria-label="Line-up canvas"
  aria-describedby="sprite-canvas-help"
  tabindex="0"
  style:cursor
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  onpointercancel={onpointerup}
  onpointerleave={() => (pointer = null)}
  {onwheel}
  {onkeydown}
>
  {#if sprites.other?.url && otherShown && !sprites.otherOnTop}
    <img class={["layer", { sharp }]} src={sprites.other.url} style={otherStyle} alt="" draggable="false" />
  {/if}
  {#if sprites.mine?.url && mineShown && place}
    <img class={["layer", { sharp }]} src={sprites.mine.url} style={mineStyle} alt="" draggable="false" />
  {/if}
  {#if sprites.other?.url && otherShown && sprites.otherOnTop}
    <img class={["layer", { sharp }]} src={sprites.other.url} style={otherStyle} alt="" draggable="false" />
  {/if}

  <svg class="overlay" {width} {height} aria-hidden="true">
    {#if boxRect}
      <rect class="box" x={boxRect.x - 0.5} y={boxRect.y - 0.5} width={boxRect.w + 1} height={boxRect.h + 1} />
      <text class="box-label" x={boxRect.x} y={boxRect.y - 6}>Game's box {fmt(box?.w ?? 0)} x {fmt(box?.h ?? 0)}</text>
    {/if}
    {#if outline.length === 4 && sprites.mine?.visible}
      <polygon class="mine" points={outline.map((c) => `${c.x},${c.y}`).join(" ")} />
      {#if turnAt && topMid}
        <line class="mine" x1={topMid.x} y1={topMid.y} x2={turnAt.x} y2={turnAt.y} />
        <circle class="handle" cx={turnAt.x} cy={turnAt.y} r="6" />
      {/if}
      {#each outline as c, i (i)}
        <rect class="handle" x={c.x - 5} y={c.y - 5} width="10" height="10" />
      {/each}
    {/if}
  </svg>

  <div class="readout" aria-hidden="true">
    {#if pointer}x {Math.floor(pointer.x)}, y {Math.floor(pointer.y)}{/if}
  </div>
</div>

<style>
  .stage {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    touch-action: none;
    user-select: none;
  }

  .stage:focus-visible {
    box-shadow: var(--qm-focus-ring);
    outline: none;
  }

  .ground-checker {
    background-color: #8f8f8f;
    background-image: conic-gradient(#b4b4b4 25%, transparent 0 50%, #b4b4b4 0 75%, transparent 0);
    background-size: 16px 16px;
  }

  .ground-dark {
    background: #1b1f1b;
  }

  .ground-light {
    background: #f2f2ee;
  }

  .layer {
    position: absolute;
    max-width: none;
    transform-origin: 50% 50%;
    pointer-events: none;
  }

  .layer.sharp {
    image-rendering: pixelated;
  }

  .overlay {
    position: absolute;
    inset: 0;
    overflow: visible;
    pointer-events: none;
  }

  .box {
    fill: none;
    stroke: #e8c25a;
    stroke-dasharray: 5 4;
    stroke-width: 1;
  }

  .box-label {
    fill: #e8c25a;
    font-family: var(--qm-font-sans);
    font-size: 11px;
    paint-order: stroke;
    stroke: #000000a0;
    stroke-width: 3px;
  }

  .mine {
    fill: none;
    stroke: #5fb4ff;
    stroke-width: 1;
  }

  .handle {
    fill: #ffffff;
    stroke: #1d5f9e;
    stroke-width: 1.5;
  }

  .readout {
    position: absolute;
    right: var(--qm-space-2);
    bottom: var(--qm-space-1);
    padding: 1px 6px;
    border-radius: var(--qm-radius-sm);
    background: #000000a0;
    color: #ffffff;
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-11);
  }

  .readout:empty {
    display: none;
  }
</style>
