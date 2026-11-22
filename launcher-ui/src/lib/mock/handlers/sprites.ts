import type { SpriteCatalog, SpriteItem, SpritePlace, SpriteRef, SpriteSaveRequest, SpriteSaved, SpriteShelf, SpriteUse, SpriteUses } from "../../bindings";
import { mods, settings } from "../fixtures";
import { fail, type Handlers } from "../state";
import { mockFileText, mockWorkspace, mockWriteFile } from "./engine";

/** The game's images in the mock: path and pixel size. */
const GAME: [string, number, number][] = [
  ["sprites/allied_fire.png", 60, 30],
  ["sprites/allied_aim.png", 60, 30],
  ["sprites/allied_walk1.png", 60, 30],
  ["sprites/allied_walk2.png", 60, 30],
  ["sprites/allied_walk3.png", 60, 30],
  ["sprites/top_m1.png", 40, 12],
  ["sprites/side_m1.png", 50, 20],
  ["sprites/rank_1.png", 18, 22],
];

const stemOf = (path: string) => (path.split("/").pop() ?? path).replace(/\.[^.]+$/, "");
const isImage = (path: string) => /\.(png|jpe?g|bmp)$/i.test(path);

function hue(text: string): number {
  let h = 0;
  for (let i = 0; i < text.length; i++) h = (h * 31 + text.charCodeAt(i)) % 360;
  return h;
}

/** A stand-in picture: a body and a head, so a misplaced one shows. */
function picture(key: string, w: number, h: number): string {
  const c = hue(key);
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">` +
    `<rect x="${w * 0.2}" y="${h * 0.35}" width="${w * 0.6}" height="${h * 0.55}" fill="hsl(${c} 45% 45%)"/>` +
    `<circle cx="${w * 0.35}" cy="${h * 0.25}" r="${Math.min(w, h) * 0.18}" fill="hsl(${(c + 40) % 360} 50% 70%)"/>` +
    `<rect x="${w * 0.55}" y="${h * 0.45}" width="${w * 0.42}" height="${Math.max(1, h * 0.08)}" fill="#222"/>` +
    `</svg>`;
  return `data:image/svg+xml;base64,${btoa(svg)}`;
}

function gameItem(path: string): SpriteItem | null {
  const g = GAME.find(([p]) => p === path);
  return g ? { sprite: { source: "game", owner: "", path }, name: stemOf(path), width: g[1], height: g[2] } : null;
}

/** A mod image's size, from its stand-in text "PNG 64x40". */
function modSize(text: string | undefined | null): [number, number] {
  const m = /(\d+)x(\d+)/.exec(text ?? "");
  return m ? [Number(m[1]), Number(m[2])] : [32, 32];
}

function folderOf(source: SpriteRef["source"], owner: string): Map<string, string | null> {
  if (source === "workspace") return mockWorkspace().get(owner) ?? fail("not_found", `No mod folder '${owner}'.`);
  if (source === "library") {
    if (!mods.some((m) => m.id === owner)) fail("not_found", `No library mod '${owner}'.`);
    return new Map([[`sprites/${owner}_badge.png`, "PNG 24x24"]]);
  }
  return fail("invalid_input", "The game is not a folder.");
}

function catalog(source: SpriteRef["source"], owner: string): SpriteCatalog {
  if (source === "game") {
    if (!settings.game_exe_path) fail("not_found", "The game's files are not read yet.");
    return { items: GAME.map(([p]) => gameItem(p) as SpriteItem), truncated: false };
  }
  const f = folderOf(source, owner);
  const items: SpriteItem[] = [];
  for (const [path, text] of f) {
    if (text === null || !isImage(path)) continue;
    const [w, h] = modSize(text);
    items.push({ sprite: { source, owner, path }, name: stemOf(path), width: w, height: h });
  }
  items.sort((a, b) => a.sprite.path.toLowerCase().localeCompare(b.sprite.path.toLowerCase()));
  return { items, truncated: false };
}

/** A `[sprites.<global>]` table of qm.toml, or the faction's `<global> = { ... }`, as text. */
function ruleText(folder: string, global: string, faction: boolean): string {
  const file = mockFileText(folder, faction ? "faction.toml" : "qm.toml") ?? "";
  if (faction) {
    const line = new RegExp(`^${global} = \\{(.*)\\}\\s*$`, "m").exec(file);
    return line ? line[1].replace(/,\s*/g, "\n") : "";
  }
  const at = file.indexOf(`[sprites.${global}]`);
  if (at < 0) return "";
  const rest = file.slice(at + `[sprites.${global}]`.length);
  const end = rest.search(/^\[/m);
  return end < 0 ? rest : rest.slice(0, end);
}

function nums(text: string, key: string): number[] | null {
  const m = new RegExp(`\\b${key}\\s*=\\s*\\[\\s*(-?[\\d.]+)\\s*,\\s*(-?[\\d.]+)\\s*\\]`).exec(text);
  return m ? [Number(m[1]), Number(m[2])] : null;
}

function use(folder: string, global: string, faction: boolean, original: SpriteItem | null, file: [number, number]): SpriteUse {
  const rule = ruleText(folder, global, faction);
  const size = nums(rule, "size");
  const bw = size ? size[0] : original ? original.width : file[0];
  const bh = size ? size[1] : original ? original.height : file[1];
  const offset = nums(rule, "offset") ?? [0, 0];
  const draw = nums(rule, "draw_size") ?? [bw, bh];
  const rotate = Number(/\brotate\s*=\s*(-?[\d.]+)/.exec(rule)?.[1] ?? 0);
  const flip = /\bflip\s*=\s*"(\w+)"/.exec(rule)?.[1] ?? "";
  const placed = /\b(offset|draw_size|rotate|flip)\b/.test(rule);
  return {
    global,
    target: faction ? "faction" : "qm",
    original,
    box_width: bw,
    box_height: bh,
    place: { x: offset[0], y: offset[1], width: draw[0], height: draw[1], rotate, flip_x: flip.includes("x"), flip_y: flip.includes("y") },
    placed,
    size_rule: size ? `size = [${size[0]}, ${size[1]}]` : null,
  };
}

function uses(folder: string, path: string): SpriteUses {
  const f = folderOf("workspace", folder);
  const text = f.get(path);
  if (text === undefined || text === null) fail("not_found", `No file '${path}' in ${folder}.`);
  const [w, h] = modSize(text);
  const file: SpriteItem = { sprite: { source: "workspace", owner: folder, path }, name: stemOf(path), width: w, height: h };
  const out: SpriteUse[] = [];
  const notes: string[] = [];
  const faction = f.get("faction.toml");
  if (faction) {
    for (const m of faction.matchAll(/(\w+) = "([^"]+)"/g)) {
      if (m[2] !== path) continue;
      const global = `allied_${m[1]}`;
      out.push(use(folder, global, true, gameItem(`sprites/${global}.png`), [w, h]));
    }
  }
  if (path.startsWith("assets/")) {
    const global = stemOf(path);
    const original = gameItem(`sprites/${global}.png`);
    if (!original) notes.push(`The game has no image named ${global}, so this one is added as a new image of that name. Pick an image to compare it with.`);
    out.push(use(folder, global, false, original, [w, h]));
  } else if (out.length === 0) {
    notes.push("The game does not load this file: only images under assets/ and the ones faction.toml names are. Pick an image to compare it with; the place cannot be saved.");
  }
  return { file, uses: out, note: notes.length > 0 ? notes.join(" ") : null };
}

const r2 = (v: number) => Math.round(v * 100) / 100;

function keys(p: SpritePlace, bw: number, bh: number): [string, string][] {
  const out: [string, string][] = [];
  if (r2(p.x) !== 0 || r2(p.y) !== 0) out.push(["offset", `[${r2(p.x)}, ${r2(p.y)}]`]);
  if (Math.abs(r2(p.width) - r2(bw)) > 0.004 || Math.abs(r2(p.height) - r2(bh)) > 0.004) out.push(["draw_size", `[${r2(p.width)}, ${r2(p.height)}]`]);
  let t = r2(p.rotate % 360);
  if (t > 180) t -= 360;
  if (t <= -180) t += 360;
  if (t !== 0) out.push(["rotate", String(t)]);
  const flip = `${p.flip_x ? "x" : ""}${p.flip_y ? "y" : ""}`;
  if (flip) out.push(["flip", `"${flip}"`]);
  return out;
}

function addFeature(folder: string, feature: string): boolean {
  const text = mockFileText(folder, "mod.json");
  if (!text || text.includes(`"${feature}"`)) return false;
  const next = text.includes('"qm_features"')
    ? text.replace(/"qm_features":\s*\[/, `"qm_features": ["${feature}", `).replace(", ]", "]")
    : text.replace("{", `{\n  "qm_features": ["${feature}"],`);
  mockWriteFile(folder, "mod.json", next);
  return true;
}

function save(folder: string, req: SpriteSaveRequest): SpriteSaved {
  const k = keys(req.place, req.box_width, req.box_height);
  const faction = req.target === "faction";
  const file = faction ? "faction.toml" : "qm.toml";
  let text = mockFileText(folder, file) ?? (faction ? fail("not_found", "The mod has no faction.toml.") : '[manifest]\nversion = "1.0.0"\n');
  let written = "";
  if (faction) {
    const line = new RegExp(`^${req.global} = .*\\n?`, "m");
    text = text.replace(line, "");
    if (k.length > 0) {
      written = `${req.global} = { ${k.map(([a, b]) => `${a} = ${b}`).join(", ")} }`;
      text = text.includes("[sprites]") ? text.replace("[sprites]\n", `[sprites]\n${written}\n`) : `${text.trimEnd()}\n\n[sprites]\n${written}\n`;
      written = `[sprites]\n${written}`;
    }
  } else {
    const head = `[sprites.${req.global}]`;
    const at = text.indexOf(head);
    let kept: string[] = [];
    if (at >= 0) {
      const rest = text.slice(at + head.length);
      const end = rest.search(/^\[/m);
      const body = end < 0 ? rest : rest.slice(0, end);
      kept = body.split("\n").filter((l) => l.trim() && !/^(offset|draw_size|rotate|flip)\b/.test(l.trim()));
      text = text.slice(0, at) + (end < 0 ? "" : rest.slice(end));
    }
    const lines = [...kept, ...k.map(([a, b]) => `${a} = ${b}`)];
    if (lines.length > 0) {
      written = `${head}\n${lines.join("\n")}`;
      text = `${text.trimEnd()}\n\n${written}\n`;
    }
  }
  mockWriteFile(folder, file, text);
  const feature = faction ? "faction_sprite_sizes" : "sprite_transforms";
  const added = k.length > 0 && addFeature(folder, feature);
  const where = faction ? `${file}, [sprites] ${req.global}` : `${file}, [sprites.${req.global}]`;
  let message = k.length === 0 ? `Saved into ${where}: no place, so the game draws your image filling the original's box.` : `Saved into ${where}. In game your image is placed as you lined it up, inside the original's box.`;
  if (added) message += ` Added "${feature}" to mod.json, so a QuarterMaster too old for it skips the mod instead of drawing the image out of place.`;
  return { file, written, message, feature_added: added ? feature : null };
}

export const handlers: Handlers = {
  engine_sprite_shelves: (args): SpriteShelf[] => {
    const folder = String(args.folder);
    const ws = mockWorkspace();
    if (!ws.has(folder)) fail("not_found", `No mod folder '${folder}'.`);
    const name = (f: string) => /"name":\s*"([^"]+)"/.exec(ws.get(f)?.get("mod.json") ?? "")?.[1] ?? f;
    const out: SpriteShelf[] = [{ source: "workspace", owner: folder, label: name(folder) }];
    if (settings.game_exe_path) out.push({ source: "game", owner: "", label: "The game" });
    for (const f of ws.keys()) if (f !== folder) out.push({ source: "workspace", owner: f, label: name(f) });
    for (const m of mods) out.push({ source: "library", owner: m.id, label: `${m.name} ${m.version}` });
    return out;
  },
  engine_sprite_catalog: (args) => catalog(args.source as SpriteRef["source"], String(args.owner)),
  engine_sprite_image: (args) => {
    const r = args.sprite as SpriteRef;
    if (r.source === "game") {
      const g = gameItem(r.path) ?? fail("not_found", `The game has no image ${r.path}.`);
      return picture(`game:${r.path}`, g.width, g.height);
    }
    if (r.path.split("/").some((p) => p === ".." || p === "")) fail("invalid_input", `'${r.path}' is not a path inside the mod.`);
    const f = folderOf(r.source, r.owner);
    const text = f.get(r.path);
    if (text === undefined || text === null) fail("not_found", `No image '${r.path}'.`);
    const [w, h] = modSize(text);
    // The text can change (a new version of the file): the picture follows it.
    return picture(`${r.owner}:${r.path}:${text}`, w, h);
  },
  engine_sprite_uses: (args) => uses(String(args.folder), String(args.path)),
  engine_sprite_save: (args) => save(String(args.folder), args.req as SpriteSaveRequest),
};
