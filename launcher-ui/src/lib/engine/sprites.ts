/**
 * The sprite line-up's geometry, in the game's pixels with the box's top-left corner at 0, 0
 * and y going down. Your image is a rectangle (x, y, width, height), turned `rotate` degrees
 * clockwise about its middle and mirrored inside itself, as the loader draws it.
 */
import type { SpriteItem, SpritePlace } from "../bindings";

export interface Point {
  x: number;
  y: number;
}

export interface Size {
  w: number;
  h: number;
}

/** The whole box, unturned: how the game shows a replacement with no place. */
export function wholeBox(box: Size): SpritePlace {
  return { x: 0, y: 0, width: box.w, height: box.h, rotate: 0, flip_x: false, flip_y: false };
}

export function samePlace(a: SpritePlace | null, b: SpritePlace | null): boolean {
  if (!a || !b) return a === b;
  const near = (p: number, q: number) => Math.abs(p - q) < 0.005;
  return (
    near(a.x, b.x) &&
    near(a.y, b.y) &&
    near(a.width, b.width) &&
    near(a.height, b.height) &&
    near(normalTurn(a.rotate), normalTurn(b.rotate)) &&
    a.flip_x === b.flip_x &&
    a.flip_y === b.flip_y
  );
}

/** A turn in -180 to 180 degrees. */
export function normalTurn(deg: number): number {
  let t = deg % 360;
  if (t > 180) t -= 360;
  if (t <= -180) t += 360;
  return t;
}

/** Where a comparison image sits: centred on the box's middle, at its own size (the game
 * draws most images centred, and the original fills the box). */
export function comparisonRect(box: Size, size: Size): { x: number; y: number; w: number; h: number } {
  return { x: (box.w - size.w) / 2, y: (box.h - size.h) / 2, w: size.w, h: size.h };
}

export function centreOf(p: SpritePlace): Point {
  return { x: p.x + p.width / 2, y: p.y + p.height / 2 };
}

function rotateBy(v: Point, deg: number): Point {
  const r = (deg * Math.PI) / 180;
  const c = Math.cos(r);
  const s = Math.sin(r);
  return { x: v.x * c - v.y * s, y: v.x * s + v.y * c };
}

/** The four corners of your image, in the game's pixels: top-left, top-right, bottom-right,
 * bottom-left of the unturned picture. */
export function corners(p: SpritePlace): Point[] {
  const c = centreOf(p);
  return [
    [-1, -1],
    [1, -1],
    [1, 1],
    [-1, 1],
  ].map(([sx, sy]) => {
    const v = rotateBy({ x: (sx * p.width) / 2, y: (sy * p.height) / 2 }, p.rotate);
    return { x: c.x + v.x, y: c.y + v.y };
  });
}

/** The spot of the turn handle: above the middle of the top edge, `gap` game pixels out. */
export function turnHandle(p: SpritePlace, gap: number): Point {
  const c = centreOf(p);
  const v = rotateBy({ x: 0, y: -p.height / 2 - gap }, p.rotate);
  return { x: c.x + v.x, y: c.y + v.y };
}

/** Whole pixels, or tenths with `fine`. */
export function snap(v: number, fine = false): number {
  return fine ? Math.round(v * 10) / 10 : Math.round(v);
}

/** Moved by `d` from where a drag started. */
export function moved(start: SpritePlace, d: Point, fine = false): SpritePlace {
  return { ...start, x: snap(start.x + d.x, fine), y: snap(start.y + d.y, fine) };
}

/**
 * Resized by dragging corner `corner` (0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left)
 * to `to`, the opposite corner staying where it is. `keepShape` keeps the width to height
 * ratio of `start`.
 */
export function resized(start: SpritePlace, corner: number, to: Point, keepShape: boolean, fine = false): SpritePlace {
  const signs = [
    [-1, -1],
    [1, -1],
    [1, 1],
    [-1, 1],
  ];
  const [sx, sy] = signs[corner];
  const fixed = corners(start)[(corner + 2) % 4];
  const d = rotateBy({ x: to.x - fixed.x, y: to.y - fixed.y }, -start.rotate);
  let w = d.x * sx;
  let h = d.y * sy;
  if (keepShape) {
    // The drag's reach along the corner's own diagonal.
    const vx = sx * start.width;
    const vy = sy * start.height;
    const k = (d.x * vx + d.y * vy) / (vx * vx + vy * vy);
    w = start.width * k;
    h = start.height * k;
  }
  const min = fine ? 0.1 : 1;
  w = Math.max(min, snap(w, fine));
  h = Math.max(min, snap(h, fine));
  const half = rotateBy({ x: (sx * w) / 2, y: (sy * h) / 2 }, start.rotate);
  const c = { x: fixed.x + half.x, y: fixed.y + half.y };
  return { ...start, width: w, height: h, x: snap(c.x - w / 2, true), y: snap(c.y - h / 2, true) };
}

/** Turned so the turn handle points at `to`; `step` snaps to that many degrees. */
export function turned(start: SpritePlace, to: Point, step = 0, fine = false): SpritePlace {
  const c = centreOf(start);
  let deg = (Math.atan2(to.y - c.y, to.x - c.x) * 180) / Math.PI + 90;
  deg = step > 0 ? Math.round(deg / step) * step : snap(deg, fine);
  return { ...start, rotate: normalTurn(deg) };
}

/** Your image at `size`, its middle where it is. */
export function sizedAround(p: SpritePlace, size: Size): SpritePlace {
  const c = centreOf(p);
  return { ...p, width: size.w, height: size.h, x: snap(c.x - size.w / 2, true), y: snap(c.y - size.h / 2, true) };
}

/** Your image's middle on `at`, its size and turn kept. */
export function centredOn(p: SpritePlace, at: Point): SpritePlace {
  return { ...p, x: snap(at.x - p.width / 2, true), y: snap(at.y - p.height / 2, true) };
}

/** The smallest rectangle around the box, a comparison and your image. */
export function bounds(box: Size, parts: { x: number; y: number; w: number; h: number }[], place: SpritePlace | null) {
  const xs = [0, box.w];
  const ys = [0, box.h];
  for (const r of parts) {
    xs.push(r.x, r.x + r.w);
    ys.push(r.y, r.y + r.h);
  }
  if (place) for (const c of corners(place)) {
    xs.push(c.x);
    ys.push(c.y);
  }
  return { x: Math.min(...xs), y: Math.min(...ys), w: Math.max(...xs) - Math.min(...xs), h: Math.max(...ys) - Math.min(...ys) };
}

/** The zoom steps, in screen pixels per game pixel. */
export const ZOOMS = [0.25, 0.5, 1, 2, 3, 4, 6, 8, 12, 16];

export function zoomStep(now: number, dir: 1 | -1): number {
  if (dir > 0) return ZOOMS.find((z) => z > now + 1e-6) ?? ZOOMS[ZOOMS.length - 1];
  return [...ZOOMS].reverse().find((z) => z < now - 1e-6) ?? ZOOMS[0];
}

/** `walk3` -> { base: "walk", n: 3 }; a name without a number at its end is its own set. */
export function numbered(name: string): { base: string; n: number | null } {
  const m = /^(.*?)(\d+)$/.exec(name);
  return m ? { base: m[1], n: Number(m[2]) } : { base: name, n: null };
}

function folderOf(path: string): string {
  const i = path.lastIndexOf("/");
  return i < 0 ? "" : path.slice(0, i).toLowerCase();
}

/** The numbered set an image belongs to (same folder, same name before its number), in
 * number order; just the image when it has no number or no others. */
export function setOf(item: SpriteItem, all: SpriteItem[]): SpriteItem[] {
  const { base, n } = numbered(item.name);
  if (n === null) return [item];
  const dir = folderOf(item.sprite.path);
  const set = all.filter((i) => {
    const m = numbered(i.name);
    return m.n !== null && m.base.toLowerCase() === base.toLowerCase() && folderOf(i.sprite.path) === dir;
  });
  set.sort((a, b) => (numbered(a.name).n ?? 0) - (numbered(b.name).n ?? 0));
  return set.length > 0 ? set : [item];
}

/** A short line for a number: whole numbers bare, else up to two decimals. */
export function fmt(v: number): string {
  return Number.isInteger(v) ? String(v) : String(Math.round(v * 100) / 100);
}

/** The drawn part of an image: the rectangle around its pixels that are not see-through, in
 * the file's own pixels, with the file's size. */
export interface Drawn {
  x: number;
  y: number;
  w: number;
  h: number;
  fileW: number;
  fileH: number;
}

/** The rectangle around the pixels whose alpha is above `min`, or null when none are. */
export function drawnBounds(alpha: ArrayLike<number>, fileW: number, fileH: number, min = 16): Drawn | null {
  let x0 = fileW;
  let y0 = fileH;
  let x1 = -1;
  let y1 = -1;
  for (let y = 0; y < fileH; y++) {
    for (let x = 0; x < fileW; x++) {
      if (alpha[y * fileW + x] > min) {
        if (x < x0) x0 = x;
        if (x > x1) x1 = x;
        if (y < y0) y0 = y;
        if (y > y1) y1 = y;
      }
    }
  }
  return x1 < 0 ? null : { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1, fileW, fileH };
}

/**
 * Your image sized and moved so its drawn part covers the comparison's drawn part. `rect` is
 * where the comparison is shown in the box. With `keepShape` one scale is used for both sides
 * (the mean of the two); without, each side is fitted on its own. The turn is taken off; the
 * mirroring stays.
 */
export function matchDrawn(
  start: SpritePlace,
  mine: Drawn,
  other: Drawn,
  rect: { x: number; y: number; w: number; h: number },
  keepShape: boolean,
): SpritePlace {
  // The comparison's drawn part, in the game's pixels.
  const kx = rect.w / other.fileW;
  const ky = rect.h / other.fileH;
  const target = { x: rect.x + other.x * kx, y: rect.y + other.y * ky, w: other.w * kx, h: other.h * ky };
  let sx = target.w / mine.w;
  let sy = target.h / mine.h;
  if (keepShape) {
    const s = Math.sqrt(sx * sy);
    sx = s;
    sy = s;
  }
  const width = Math.max(1, Math.round(mine.fileW * sx));
  const height = Math.max(1, Math.round(mine.fileH * sy));
  // After rounding, the scale the picture is really drawn at.
  const rx = width / mine.fileW;
  const ry = height / mine.fileH;
  // Mirrored, the drawn part sits on the other side of the picture.
  const ax = start.flip_x ? mine.fileW - mine.x - mine.w : mine.x;
  const ay = start.flip_y ? mine.fileH - mine.y - mine.h : mine.y;
  // Its middle on the target's middle.
  const x = target.x + target.w / 2 - (ax + mine.w / 2) * rx;
  const y = target.y + target.h / 2 - (ay + mine.h / 2) * ry;
  return { ...start, x: Math.round(x), y: Math.round(y), width, height, rotate: 0 };
}

/** The scale a file seems drawn at against the game's image: 2 for a file twice its size each
 * way, up to 4; 1 when it is not a whole multiple. */
export function guessFileScale(file: Size, game: Size): number {
  for (const k of [2, 3, 4]) {
    const near = (a: number, b: number) => Math.abs(a - b) <= Math.max(1, b * 0.03);
    if (near(file.w, game.w * k) && near(file.h, game.h * k)) return k;
  }
  return 1;
}
