<script lang="ts">
  import type { Action } from "svelte/action";
  import type { Attachment } from "svelte/attachments";
  import { dndzone, type DndEvent, type DndZoneAttributes, type Options } from "svelte-dnd-action";
  import { IconGrip } from "../../../components/icons";
  import { Badge, Button, Menu } from "../../../components/ui";
  import type { FixDraft, SharedStack, StackBlock } from "../../../lib/bindings";
  import { blockLabel, canOrder, moveChoices, moveToStackRule, reasonLabel, reasonTone, stackHeading, wasText } from "../../../lib/patches/stacks";

  interface Props {
    stack: SharedStack;
    modName: (id: string) => string;
    canMutate: boolean;
    /** A move to preview: `draft` puts block `patchId` below or above another block. */
    onmove: (stack: SharedStack, patchId: string, draft: FixDraft) => void;
  }

  let { stack, modName, canMutate, onmove }: Props = $props();

  type Row = { id: string; b: StackBlock };

  const rows = $derived<Row[]>(stack.blocks.map((b) => ({ id: b.patch_id, b })));
  let dragItems = $state<Row[] | null>(null);
  let armed = $state(false);
  const heading = $derived(stackHeading(stack.file_line, stack.anchor_text, stack.side, stack.blocks));
  const movable = $derived(canMutate && canOrder(stack));

  function move(from: number, to: number) {
    if (!movable) return;
    const draft = moveToStackRule(stack, from, to);
    if (draft) onmove(stack, stack.blocks[from].patch_id, draft);
  }

  function onkeydown(e: KeyboardEvent, i: number) {
    if (!e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
    e.preventDefault();
    move(i, e.key === "ArrowUp" ? i - 1 : i + 1);
  }

  const options = $derived<Options<Row>>({
    items: dragItems ?? rows,
    flipDurationMs: 0,
    dragDisabled: !armed || !movable,
    zoneTabIndex: -1,
    zoneItemTabIndex: 0,
    autoAriaDisabled: true,
    dropTargetStyle: {},
    // One zone per line and side: a block never leaves its stack.
    type: `qm-stack:${stack.stack_key}`,
  });

  let zoneApi: { update: (o: Options<Row>) => void } | null = null;
  const zone: Action<HTMLElement, Options<Row>, DndZoneAttributes<Row>> = (node, opts) => {
    const z = dndzone(node, opts) as { update: (o: Options<Row>) => void; destroy: () => void };
    zoneApi = z;
    return { update: (o) => z.update(o), destroy: () => z.destroy() };
  };

  // Pointer drag only from the grip, like the order view; Alt+Up/Down is the keyboard path.
  const handle: Attachment<HTMLElement> = (el) => {
    const arm = () => {
      if (!movable) return;
      armed = true;
      zoneApi?.update({ ...options, dragDisabled: false });
      window.addEventListener("mouseup", () => (armed = false), { once: true });
    };
    el.addEventListener("mousedown", arm);
    return () => el.removeEventListener("mousedown", arm);
  };

  function onfinalize(e: CustomEvent<DndEvent<Row>>) {
    const id = e.detail.info.id;
    const from = rows.findIndex((r) => r.id === id);
    const to = e.detail.items.findIndex((r) => r.id === id);
    dragItems = null;
    armed = false;
    if (from !== -1 && to !== -1) move(from, to);
  }

  function choices(i: number) {
    const b = stack.blocks[i];
    return moveChoices(stack, i, modName).map((c) => ({
      label: c.label,
      disabled: c.disabled,
      disabledReason: c.disabled ? "It is there now" : undefined,
      onselect: () => onmove(stack, b.patch_id, c.draft),
    }));
  }
</script>

<article class="stack" aria-label={heading}>
  <h5 class="heading">{heading}</h5>
  <p class="qm-muted site">{stack.site_label}</p>
  <ol
    aria-label={`Code ${stack.side} line ${stack.file_line}`}
    use:zone={options}
    onconsider={(e) => (dragItems = e.detail.items)}
    {onfinalize}
  >
    {#each dragItems ?? rows as r, j (r.id)}
      {@const b = r.b}
      {@const i = rows.findIndex((x) => x.id === r.id)}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <li class="row" tabindex="0" onkeydown={(e) => onkeydown(e, i)}>
        <div class="inner">
          {#if b.locked_reason || !canOrder(stack)}
            <span class="grip-space" aria-hidden="true"></span>
          {:else}
            <span class={["handle", { off: !movable }]} aria-hidden="true" {@attach handle}><IconGrip /></span>
          {/if}
          <div class="info">
            <span class="name">{b.patch_name || b.patch_id}</span>
            <span class="qm-muted">{blockLabel(b, stack.blocks, modName)}</span>
            <span class="qm-muted lines">{b.first_line === b.last_line ? `line ${b.first_line}` : `lines ${b.first_line}-${b.last_line}`}</span>
            <!-- "Older order" only for a block that kept its older place. -->
            {#if b.reason || (stack.mode !== "legacy" && stack.legacy.indexOf(b.patch_id) === j)}
              <Badge tone={reasonTone(b.reason)}>{reasonLabel(b.reason, b.mod_id, modName)}</Badge>
            {/if}
            {#if b.locked_reason}
              <span class="qm-muted locked">{b.locked_reason}</span>
            {/if}
          </div>
          {#if !b.locked_reason && canOrder(stack) && stack.blocks.length > 1}
            <Menu items={choices(i)} align="end">
              {#snippet trigger({ props })}
                <Button size="sm" variant="ghost" disabled={!movable} {...props}>Move...</Button>
              {/snippet}
            </Menu>
          {/if}
        </div>
      </li>
    {/each}
  </ol>
  {#if stack.moved}
    <p class="qm-muted was">{wasText(stack.legacy, stack.blocks, modName)}</p>
  {/if}
</article>

<style>
  .stack {
    padding: var(--qm-space-2) var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-panel);
  }

  p {
    margin: 0;
  }

  .heading {
    margin: 0;
    font-size: var(--qm-text-13);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .site,
  .was,
  .lines,
  .locked {
    font-size: var(--qm-text-12);
  }

  ol {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    margin: var(--qm-space-2) 0;
    padding-left: var(--qm-space-5);
  }

  .row {
    display: list-item;
    border-radius: var(--qm-radius-sm);
  }

  .row:focus-visible {
    outline: none;
    box-shadow: var(--qm-focus-ring);
  }

  .inner {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .info {
    display: flex;
    flex: 1;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-1) var(--qm-space-2);
    min-width: 0;
  }

  .handle,
  .grip-space {
    display: inline-flex;
    flex: none;
    width: 16px;
  }

  .handle {
    color: var(--qm-text-muted);
    cursor: grab;
  }

  .handle.off {
    cursor: default;
    opacity: 0.5;
  }

  .name {
    font-weight: 600;
  }
</style>
