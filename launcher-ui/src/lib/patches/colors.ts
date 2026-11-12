/** How many mod colours the tokens define (`--qm-attr-1` to `--qm-attr-8`). */
export const MOD_COLOURS = 8;

function fnv1a(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h;
}

/**
 * A colour number (1 to 8) per mod. Each mod starts from its name's hash and takes the next free
 * colour, in name order, so the same mods always get the same colours whatever order they come in.
 * Past eight mods the colours repeat; mod names are always shown with them.
 */
export function assignModColours(modIds: string[]): Map<string, number> {
  const out = new Map<string, number>();
  let taken = new Set<number>();
  for (const id of [...new Set(modIds)].sort()) {
    if (taken.size === MOD_COLOURS) taken = new Set();
    let slot = fnv1a(id) % MOD_COLOURS;
    while (taken.has(slot)) slot = (slot + 1) % MOD_COLOURS;
    taken.add(slot);
    out.set(id, slot + 1);
  }
  return out;
}

export function modColour(n: number): string {
  return `var(--qm-attr-${n})`;
}
