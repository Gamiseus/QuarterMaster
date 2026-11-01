<script lang="ts">
  import { tick, untrack } from "svelte";
  import type { Action } from "svelte/action";
  import { flip } from "svelte/animate";
  import type { Attachment } from "svelte/attachments";
  import {
    dndzone,
    SHADOW_ITEM_MARKER_PROPERTY_NAME,
    type DndEvent,
    type DndZoneAttributes,
    type Options,
  } from "svelte-dnd-action";
  import type { ModSummary, ProfileSeparator } from "../../lib/bindings";
  import { rowKey, type OrderRow } from "../../lib/modlist/order";
  import type { ModStatus } from "../../lib/modlist/status";
  import { dropTarget, sectionCounts, stepTarget } from "../../lib/modlist/view";
  import { selection, type SelectMode } from "../../lib/stores/selection.svelte";
  import type { ModListController } from "./modlist-controller.svelte";
  import ModRow from "./ModRow.svelte";
  import SeparatorRow from "./SeparatorRow.svelte";

  interface Props {
    controller: ModListController;
    visible: OrderRow[];
    modsById: Map<string, ModSummary>;
    status: Map<string, ModStatus>;
    positions: Map<string, { index: number; reasons: string[] }>;
    moved: Set<string>;
    canMutate: boolean;
    oneditseparator: (sep: ProfileSeparator) => void;
    onremoveseparator: (sep: ProfileSeparator) => void;
    /** Keyboard cursor row key (bindable; the screen inserts new separators before it). */
    cursor?: string | null;
  }

  let {
    controller,
    visible,
    modsById,
    status,
    positions,
    moved,
    canMutate,
    oneditseparator,
    onremoveseparator,
    cursor = $bindable(null),
  }: Props = $props();

  /** Matches --qm-dur-fast (120ms) in tokens.css; dnd needs the number. */
  const FLIP_MS = 120;
  const reducedMotion = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  const flipMs = reducedMotion ? 0 : FLIP_MS;

  type Item = { id: string; row: OrderRow; [SHADOW_ITEM_MARKER_PROPERTY_NAME]?: boolean };

  let container = $state<HTMLElement | null>(null);
  let dragItems = $state<Item[] | null>(null);
  let armed = $state(false);
  /** Focus was in the list; restore it when a re-render drops it. */
  let keepFocus = false;

  const keys = $derived(visible.map(rowKey));
  const items = $derived<Item[]>(dragItems ?? visible.map((row) => ({ id: rowKey(row), row })));
  const activeKey = $derived(cursor !== null && keys.includes(cursor) ? cursor : (keys[0] ?? null));
  const counts = $derived(sectionCounts(controller.rows));

  // Pointer drag only: the zone stays drag-disabled (so dnd's own Space/Enter keyboard drag
  // never runs) until a handle is pressed. Alt+Up/Down is the keyboard path.
  const options = $derived<Options<Item>>({
    items,
    flipDurationMs: flipMs,
    dragDisabled: !armed || !canMutate,
    zoneTabIndex: -1,
    zoneItemTabIndex: -1,
    autoAriaDisabled: true,
    dropTargetStyle: {},
    type: "qm-mod-rows",
  });

  let zoneApi: { update: (o: Options<Item>) => void } | null = null;
  const zone: Action<HTMLElement, Options<Item>, DndZoneAttributes<Item>> = (node, opts) => {
    const z = dndzone(node, opts) as { update: (o: Options<Item>) => void; destroy: () => void };
    zoneApi = z;
    return {
      update: (o) => z.update(o),
      destroy: () => {
        z.destroy();
        zoneApi = null;
      },
    };
  };

  function disarm() {
    armed = false;
  }

  /** Native listener: must enable the zone before the mousedown reaches the row wrapper. */
  const handle: Attachment<HTMLElement> = (el) => {
    const arm = () => {
      if (!canMutate) return;
      armed = true;
      zoneApi?.update({ ...options, dragDisabled: false });
      window.addEventListener("mouseup", disarm, { once: true });
      window.addEventListener("touchend", disarm, { once: true });
    };
    el.addEventListener("mousedown", arm);
    el.addEventListener("touchstart", arm, { passive: true });
    return () => {
      el.removeEventListener("mousedown", arm);
      el.removeEventListener("touchstart", arm);
    };
  };

  function onconsider(e: CustomEvent<DndEvent<Item>>) {
    dragItems = e.detail.items;
  }

  function onfinalize(e: CustomEvent<DndEvent<Item>>) {
    const finalKeys = e.detail.items.map((i) => i.id);
    const dragged = e.detail.info.id;
    dragItems = null;
    armed = false;
    if (!keys.includes(dragged)) return;
    if (!selection.selected.has(dragged)) selection.select(dragged, "replace", keys);
    cursor = dragged;
    const group = targets(dragged);
    void controller.move(group, dropTarget(controller.rows, group, finalKeys, dragged, controller.locked));
  }

  function rowEl(key: string): HTMLElement | null {
    for (const el of container?.querySelectorAll<HTMLElement>('[role="row"]') ?? []) {
      if (el.dataset.key === key) return el;
    }
    return null;
  }

  function focusKey(key: string) {
    cursor = key;
    rowEl(key)?.focus();
  }

  // Keyed re-ordering can drop focus from a moved row; put it back on the cursor row.
  $effect(() => {
    void items;
    untrack(() => {
      const lost = document.activeElement === null || document.activeElement === document.body;
      if (keepFocus && lost && activeKey) tick().then(() => rowEl(activeKey)?.focus());
    });
  });

  /** Rows an action applies to: the visible part of the selection when the row is in it, else the row. */
  function targets(key: string): string[] {
    if (!selection.selected.has(key)) return [key];
    return keys.filter((k) => selection.selected.has(k));
  }

  function separatorOf(key: string): ProfileSeparator | null {
    const row = visible.find((r) => rowKey(r) === key);
    return row?.kind === "separator" ? row.sep : null;
  }

  function toggleRow(key: string) {
    const sep = separatorOf(key);
    if (sep) {
      void controller.updateSeparator(sep.id, { collapsed: !sep.collapsed });
      return;
    }
    const ids = targets(key).filter((k) => modsById.has(k));
    void controller.setEnabled(ids, !controller.enabled[key]);
  }

  async function step(key: string, dir: -1 | 1) {
    if (!canMutate) return;
    const group = targets(key);
    const to = stepTarget(controller.rows, group, keys, dir, controller.locked);
    if (to === null) return;
    void controller.move(group, to);
    await tick();
    focusKey(key);
  }

  function goTo(key: string, mode: SelectMode | null) {
    focusKey(key);
    if (mode) selection.select(key, mode, keys);
  }

  function onkeydown(e: KeyboardEvent) {
    const el = e.target as HTMLElement;
    if (el.getAttribute("role") !== "row" || !el.dataset.key) return;
    const key = el.dataset.key;
    const idx = keys.indexOf(key);
    const ctrl = e.ctrlKey || e.metaKey;
    const mode: SelectMode | null = e.shiftKey ? "range" : ctrl ? null : "replace";
    switch (e.key) {
      case "ArrowDown":
      case "ArrowUp": {
        e.preventDefault();
        const dir = e.key === "ArrowDown" ? 1 : -1;
        if (e.altKey) void step(key, dir);
        else if (keys[idx + dir]) goTo(keys[idx + dir], mode);
        break;
      }
      case "Home":
      case "End": {
        e.preventDefault();
        const next = e.key === "Home" ? keys[0] : keys[keys.length - 1];
        if (next) goTo(next, mode);
        break;
      }
      case " ": {
        e.preventDefault();
        if (ctrl) selection.select(key, "toggle", keys);
        else if (e.shiftKey) selection.select(key, "range", keys);
        else if (canMutate) toggleRow(key);
        break;
      }
      case "Enter": {
        const sep = separatorOf(key);
        if (sep && canMutate) {
          e.preventDefault();
          oneditseparator(sep);
        }
        break;
      }
      case "Delete": {
        const sep = separatorOf(key);
        if (sep && canMutate) {
          e.preventDefault();
          onremoveseparator(sep);
        }
        break;
      }
    }
  }

  function onclick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    const row = target.closest<HTMLElement>('[role="row"]');
    const key = row?.dataset.key;
    if (!row || !key) return;
    cursor = key;
    if (target.closest("button, input, a")) return;
    const mode: SelectMode = e.shiftKey ? "range" : e.ctrlKey || e.metaKey ? "toggle" : "replace";
    selection.select(key, mode, keys);
    row.focus();
  }

  function ondblclick(e: MouseEvent) {
    const key = (e.target as HTMLElement).closest<HTMLElement>('[role="row"]')?.dataset.key;
    const sep = key ? separatorOf(key) : null;
    if (sep && canMutate && !(e.target as HTMLElement).closest("button")) oneditseparator(sep);
  }

  function onfocusin(e: FocusEvent) {
    keepFocus = true;
    const key = (e.target as HTMLElement).closest<HTMLElement>('[role="row"]')?.dataset.key;
    if (key) cursor = key;
  }

  function onfocusout(e: FocusEvent) {
    const next = e.relatedTarget as Node | null;
    if (next && !container?.contains(next)) keepFocus = false;
  }
</script>

<div
  bind:this={container}
  class="grid"
  role="grid"
  tabindex="-1"
  aria-label="Mods"
  aria-multiselectable="true"
  use:zone={options}
  {onconsider}
  {onfinalize}
  {onkeydown}
  {onclick}
  {ondblclick}
  {onfocusin}
  {onfocusout}
>
  {#each items as item (item.id)}
    {@const row = item.row}
    <div class={["item", { shadow: item[SHADOW_ITEM_MARKER_PROPERTY_NAME] }]} role="presentation" animate:flip={{ duration: flipMs }}>
      {#if row.kind === "separator"}
        <SeparatorRow
          sep={row.sep}
          count={counts.get(row.sep.id) ?? 0}
          selected={selection.selected.has(item.id)}
          active={activeKey === item.id}
          {canMutate}
          {handle}
          ontogglecollapse={() => controller.updateSeparator(row.sep.id, { collapsed: !row.sep.collapsed })}
          onedit={() => oneditseparator(row.sep)}
          onremove={() => onremoveseparator(row.sep)}
        />
      {:else}
        {@const mod = modsById.get(row.id)}
        {#if mod}
          <ModRow
            {mod}
            enabled={controller.enabled[row.id] ?? mod.enabled}
            status={status.get(row.id)}
            position={positions.has(row.id) ? (positions.get(row.id)?.index ?? 0) + 1 : null}
            movedReasons={moved.has(row.id) ? (positions.get(row.id)?.reasons ?? []) : null}
            locked={controller.locked.has(row.id)}
            selected={selection.selected.has(item.id)}
            active={activeKey === item.id}
            {canMutate}
            {handle}
            onenabledchange={(v) => controller.setEnabled([row.id], v)}
            ontogglelock={() => controller.toggleLock(row.id)}
          />
        {/if}
      {/if}
    </div>
  {/each}
</div>

<style>
  .grid {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-md);
    overflow: hidden;
    background: var(--qm-panel);
  }

  .item:last-child :global([role="row"]) {
    border-bottom: 0;
  }

  .item.shadow {
    opacity: 0.4;
    outline: 1px dashed var(--qm-accent);
    outline-offset: -1px;
  }
</style>
