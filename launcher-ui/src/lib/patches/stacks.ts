// Words for shared lines: lines where code from several patches sits on one side (plan 2e §5.5).
import type { FixDraft, SharedStack, StackBlock, StackMode, StackReason, StackSide } from "../bindings";

/** The Patches tab's "Code at shared lines" choices, recommended first. */
export const STACK_MODES: { value: StackMode; label: string }[] = [
  { value: "dependencies", label: "Dependencies first (recommended)" },
  { value: "load_order", label: "Load order" },
  { value: "legacy", label: "Older order" },
];

/** What each mode does, in one sentence. */
export const MODE_TEXT: Record<StackMode, string> = {
  dependencies: "Code from a mod goes below the code of the mods it depends on. Other code keeps the older order.",
  load_order: "Code at each shared line runs in load order.",
  legacy: "Code at each shared line keeps the order QuarterMaster used before: after a line, the mod loaded last comes first.",
};

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** "5 mods add code after it", or "2 patches add code after it" when one mod has several. */
export function stackCount(blocks: StackBlock[], side: StackSide): string {
  const mods = new Set(blocks.map((b) => b.mod_id)).size;
  const who = mods === blocks.length ? plural(mods, "mod adds", "mods add") : plural(blocks.length, "patch adds", "patches add");
  return `${who} code ${side} it`;
}

/** `Line 16: require "includes/directives" (5 mods add code after it)`. */
export function stackHeading(line: number, text: string, side: StackSide, blocks: StackBlock[]): string {
  return `Line ${line}: ${text} (${stackCount(blocks, side)})`;
}

/** A block's name in a stack: the mod, plus the patch when the mod has more than one block there. */
export function blockLabel(b: StackBlock, blocks: StackBlock[], modName: (id: string) => string): string {
  const shared = blocks.filter((x) => x.mod_id === b.mod_id).length > 1;
  return shared ? `${modName(b.mod_id)} (${b.patch_name || b.patch_id})` : modName(b.mod_id);
}

/** "Was: GDumper, Tactical Overhaul, GCore" for a stack whose order moved. */
export function wasText(legacy: string[], blocks: StackBlock[], modName: (id: string) => string): string {
  const names = legacy.map((id) => {
    const b = blocks.find((x) => x.patch_id === id);
    return b ? blockLabel(b, blocks, modName) : id;
  });
  return `Was: ${names.join(", ")}`;
}

/** The badge for why a block sits where it does; `mod` is the block's own mod. */
export function reasonLabel(reason: StackReason | null | undefined, mod: string, modName: (id: string) => string): string {
  if (!reason) return "Older order";
  switch (reason.kind) {
    case "below_dependency":
      return `Below ${modName(reason.of)}: ${modName(mod)} loads after ${modName(reason.of)}`;
    case "above_dependent":
      return `Above ${modName(reason.of)}: ${modName(reason.of)} loads after ${modName(mod)}`;
    case "load_order":
      return "Load order";
    case "unsatisfiable":
      return `Below ${modName(reason.below)}: no place is also above ${modName(reason.above)}`;
    case "guard_clamped":
      return "Kept in place: next site";
    case "user_rule":
      return "Moved by you";
    case "compat_rule":
      return `Moved by ${modName(reason.mod_id)}`;
    case "author_hint":
      if (reason.of === mod) return `${modName(mod)} asks to run ${reason.hint}`;
      return reason.hint === "first"
        ? `Below ${modName(reason.of)}: it asks to run first`
        : `Above ${modName(reason.of)}: it asks to run last`;
  }
}

/** Badge tone: a dependency move is information, a conflict between rules is a warning. */
export function reasonTone(reason: StackReason | null | undefined): "info" | "warn" | "neutral" {
  if (!reason) return "neutral";
  return reason.kind === "unsatisfiable" ? "warn" : "info";
}

/** A StackRule draft (D55): at the stack's line and side, `patchId`'s block goes below `below`'s block or above `above`'s. */
export function stackRuleDraft(
  s: Pick<SharedStack, "site" | "side">,
  patchId: string,
  place: { below: string } | { above: string },
): FixDraft {
  return {
    change: {
      kind: "stack_rule",
      patch_id: patchId,
      site: s.site,
      side: s.side,
      below_patch_id: "below" in place ? place.below : null,
      above_patch_id: "above" in place ? place.above : null,
    },
    label: "",
    source: "order_view",
  };
}

/** Whether a stack's blocks can be moved at all: its line is a line of the game's file. */
export function canOrder(s: Pick<SharedStack, "site">): boolean {
  return s.site.context.trim() !== "";
}

/**
 * The one StackRule that puts the block at `from` where `to` is (mirrors `order.ts` `moveToRule`):
 * below the block now at `to` when moving down, above it when moving up. Null when nothing moves,
 * the line cannot be ordered, or either block is locked.
 */
export function moveToStackRule(s: Pick<SharedStack, "site" | "side" | "blocks">, from: number, to: number): FixDraft | null {
  const n = s.blocks.length;
  if (from === to || from < 0 || to < 0 || from >= n || to >= n || !canOrder(s)) return null;
  const me = s.blocks[from];
  const other = s.blocks[to];
  if (me.locked_reason) return null;
  return stackRuleDraft(s, me.patch_id, to > from ? { below: other.patch_id } : { above: other.patch_id });
}

/**
 * The row action's choices for block `index`: "Move below <block>'s block" for each other block,
 * and to the top. A rule only bounds the block's place (`stack_slot_with`): "below X" for a block
 * X above it already holds, so a choice there is built from the place it names, as a drag is:
 * above the block now directly under X.
 */
export function moveChoices(
  s: Pick<SharedStack, "site" | "side" | "blocks">,
  index: number,
  modName: (id: string) => string,
): { label: string; draft: FixDraft; disabled: boolean }[] {
  const me = s.blocks[index];
  if (!me || me.locked_reason || !canOrder(s)) return [];
  const out = s.blocks.flatMap((b, i) => {
    if (i === index) return [];
    const place = i < index ? { above: s.blocks[i + 1].patch_id } : { below: b.patch_id };
    return [{ label: `Move below ${blockLabel(b, s.blocks, modName)}'s block`, draft: stackRuleDraft(s, me.patch_id, place), disabled: i === index - 1 }];
  });
  const top = s.blocks[0];
  if (index > 0 && top) {
    out.unshift({ label: `Move above ${blockLabel(top, s.blocks, modName)}'s block`, draft: stackRuleDraft(s, me.patch_id, { above: top.patch_id }), disabled: false });
  }
  return out;
}
