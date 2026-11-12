<script lang="ts">
  import type { Attachment } from "svelte/attachments";
  import type { Action } from "svelte/action";
  import { dndzone, type DndEvent, type DndZoneAttributes, type Options } from "svelte-dnd-action";
  import { IconGrip, IconLock } from "../../../components/icons";
  import { Badge, Button, Callout, Checkbox, Dialog, Skeleton, Tooltip } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type InferredEdge, type OrderRule, type PatchDetail, type PreviewResult } from "../../../lib/bindings";
  import { assignModColours } from "../../../lib/patches/colors";
  import { edgeLanes, laneSegments } from "../../../lib/patches/groups";
  import { authorEdges, crossedEdges, moveToRule, type AuthorEdge, type OrderItem } from "../../../lib/patches/order";
  import ImpactNotes from "./ImpactNotes.svelte";

  interface Props {
    file: string;
    /** The file's patch ids (any order; the view sorts by the engine's order). */
    patchIds: string[];
    modNames: Record<string, string>;
    canMutate: boolean;
    /** The file's current result, such as "3 applied, 1 broken". */
    summary: string;
    /** Anchors on another patch's code in this file, drawn as dashed edges (G10). */
    edges?: InferredEdge[];
  }

  let { file, patchIds, modNames, canMutate, summary, edges: inferred = [] }: Props = $props();

  type Row = { id: string; d: PatchDetail; item: OrderItem };

  let rows = $state<Row[] | null>(null);
  let dragItems = $state<Row[] | null>(null);
  let armed = $state(false);
  type Preview = { status: "loading" } | { status: "ready"; data: PreviewResult } | { status: "error"; message: string };
  /** A move waiting for Apply: every order change is previewed first. */
  let pending = $state<{ rule: OrderRule; crossed: AuthorEdge[]; preview: Preview } | null>(null);
  let confirmOpen = $state(false);
  /** "I understand" for an order that runs a mod before one it depends on (D39). */
  let understood = $state(false);
  let busy = $state(false);
  let seq = 0;
  let previewSeq = 0;

  const mod = (id: string) => modNames[id] ?? id;
  const list = $derived(rows ?? []);
  const items = $derived(list.map((r) => r.item));
  const edges = $derived(authorEdges(items));
  const colours = $derived(assignModColours(list.map((r) => r.item.mod)));
  const nameOf = (id: string) => list.find((r) => r.id === id)?.item.name ?? id;
  /** Inferred edges by row; while a row is dragged they are hidden, as the rows move. */
  const lanes = $derived(dragItems ? [] : edgeLanes(list.map((r) => r.id), inferred, nameOf));

  async function load() {
    const mine = ++seq;
    const ds = await Promise.all(
      patchIds.map((id) => call(commands.patchDetail(id), { expect: ["not_found", "conflict"] }).catch(() => undefined)),
    );
    if (mine !== seq) return;
    const at = (d: PatchDetail) => d.positions.find((p) => p.target_file === file)?.index ?? Number.MAX_SAFE_INTEGER;
    rows = ds
      .filter((d): d is PatchDetail => d !== undefined)
      .sort((a, b) => at(a) - at(b))
      .map((d) => ({
        id: d.patch_id,
        d,
        item: { id: d.patch_id, mod: d.mod_id, name: d.name ?? d.patch_id, priority: d.priority, before: d.declared_before, after: d.declared_after },
      }));
  }

  $effect(() => {
    void file;
    void patchIds;
    void load();
  });

  async function preview(rule: OrderRule) {
    const mine = ++previewSeq;
    if (pending) pending.preview = { status: "loading" };
    try {
      const data = await call(commands.simulationPreview({ order_rules: [rule], focus_file: file }), { expect: ["cancelled", "conflict"] });
      if (mine === previewSeq && pending) pending.preview = { status: "ready", data };
    } catch (e) {
      if (mine === previewSeq && pending) pending.preview = { status: "error", message: e instanceof Error ? e.message : String(e) };
    }
  }

  function move(from: number, to: number) {
    const rule = moveToRule(items, from, to);
    if (!rule || !canMutate) return;
    pending = { rule, crossed: crossedEdges(items, from, to), preview: { status: "loading" } };
    understood = false;
    confirmOpen = true;
    void preview(rule);
  }

  async function apply() {
    if (!pending || busy) return;
    busy = true;
    try {
      await call(commands.patchOrderSet(pending.rule));
      confirmOpen = false;
    } catch {
      // call() showed the error.
    } finally {
      busy = false;
    }
  }

  function onkeydown(e: KeyboardEvent, i: number) {
    if (!e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
    e.preventDefault();
    move(i, e.key === "ArrowUp" ? i - 1 : i + 1);
  }

  const options = $derived<Options<Row>>({
    items: dragItems ?? list,
    flipDurationMs: 0,
    dragDisabled: !armed || !canMutate,
    zoneTabIndex: -1,
    zoneItemTabIndex: 0,
    autoAriaDisabled: true,
    dropTargetStyle: {},
    type: `qm-patch-order:${file}`,
  });

  let zoneApi: { update: (o: Options<Row>) => void } | null = null;
  const zone: Action<HTMLElement, Options<Row>, DndZoneAttributes<Row>> = (node, opts) => {
    const z = dndzone(node, opts) as { update: (o: Options<Row>) => void; destroy: () => void };
    zoneApi = z;
    return { update: (o) => z.update(o), destroy: () => z.destroy() };
  };

  // Pointer drag only from the handle, like the mod list; Alt+Up/Down is the keyboard path.
  const handle: Attachment<HTMLElement> = (el) => {
    const arm = () => {
      if (!canMutate) return;
      armed = true;
      zoneApi?.update({ ...options, dragDisabled: false });
      window.addEventListener("mouseup", () => (armed = false), { once: true });
    };
    el.addEventListener("mousedown", arm);
    return () => el.removeEventListener("mousedown", arm);
  };

  function onfinalize(e: CustomEvent<DndEvent<Row>>) {
    const id = e.detail.info.id;
    const from = list.findIndex((r) => r.id === id);
    const to = e.detail.items.findIndex((r) => r.id === id);
    dragItems = null;
    armed = false;
    if (from !== -1 && to !== -1) move(from, to);
  }

  function lockText(id: string): string[] {
    return edges.flatMap((e) =>
      e.then === id ? [`Must run after ${nameOf(e.first)} (set by ${mod(e.setBy)})`] :
      e.first === id ? [`Must run before ${nameOf(e.then)} (set by ${mod(e.setBy)})`] : [],
    );
  }
</script>

<div class="order-view">
  <p class="qm-muted summary" role="status">{summary}</p>
  {#if rows === null}
    <div aria-busy="true" aria-label="Loading the order"><Skeleton lines={4} /></div>
  {:else}
    <ul
      class="rows"
      aria-label={`Patch order in ${file}`}
      use:zone={options}
      onconsider={(e) => (dragItems = e.detail.items)}
      {onfinalize}
    >
      {#each dragItems ?? list as r, i (r.id)}
        {@const locks = lockText(r.id)}
        <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
        <li class={["row", `c${colours.get(r.item.mod) ?? 0}`]} tabindex="0" onkeydown={(e) => onkeydown(e, i)}>
          <span class="handle" aria-hidden="true" {@attach handle}><IconGrip /></span>
          {#if lanes.length > 0}
            <span class="lanes">
              {#each laneSegments(lanes, i) as seg, k (k)}
                {@const end = lanes.find((l) => l.lane === k && (l.user === i || l.source === i))}
                <span class={["lane", seg]}>
                  {#if end && seg !== "through"}
                    <Tooltip text={end.text}>
                      {#snippet children({ props })}
                        <span {...props} class="dot" role="img" aria-label={end.text} tabindex="-1"></span>
                      {/snippet}
                    </Tooltip>
                  {/if}
                </span>
              {/each}
            </span>
          {/if}
          <span class="n">{i + 1}</span>
          <span class="name">{r.item.name}</span>
          <span class="qm-muted mod">{mod(r.item.mod)}</span>
          <span class="qm-muted prio">Priority {r.item.priority}</span>
          {#each locks as text (text)}
            <Tooltip {text}>
              {#snippet children({ props })}
                <span {...props} class="lock" role="img" aria-label={text} tabindex="-1"><IconLock /></span>
              {/snippet}
            </Tooltip>
          {/each}
          {#if r.d.profile_rules.some((x) => x.patch === r.id)}
            <Badge tone="info">Moved by you</Badge>
            <Button size="sm" variant="ghost" disabled={!canMutate} onclick={() => call(commands.patchOrderReset(r.id)).catch(() => {})}>Reset</Button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

{#if pending}
  {@const p = pending}
  {@const crossed = p.crossed.length > 0}
  {@const breaks = p.preview.status === "ready" && p.preview.data.impact.breaks_file}
  {@const warnings = p.preview.status === "ready" ? (p.preview.data.impact.relation_warnings ?? []) : []}
  <Dialog
    bind:open={confirmOpen}
    title={crossed ? `Override ${mod(p.crossed[0].setBy)}'s rule for this profile?` : `Move ${nameOf(p.rule.patch)}?`}
    description={crossed
      ? p.crossed.map((e) => `${nameOf(e.then)} must run after ${nameOf(e.first)}.`).join(" ") + " The mod's author set this; moving the patch may break it."
      : `${nameOf(p.rule.patch)} will run ${p.rule.before?.length ? `before ${p.rule.before.map(nameOf).join(", ")}` : `after ${(p.rule.after ?? []).map(nameOf).join(", ")}`} in ${file}.`}
    initialFocus="content"
    size="lg"
    dismissible={!busy}
  >
    {#if p.preview.status === "loading"}
      <div aria-busy="true" aria-label="Checking what changes"><Skeleton lines={3} /></div>
    {:else if p.preview.status === "error"}
      <Callout tone="danger" title="Could not check what this changes.">
        {p.preview.message}
        {#snippet action()}<Button size="sm" onclick={() => preview(p.rule)}>Retry</Button>{/snippet}
      </Callout>
    {:else}
      <ImpactNotes impact={p.preview.data.impact} {modNames} name={nameOf} />
      {#if warnings.length > 0}
        <Callout tone="warn" title="This order may break a mod.">
          <ul class="warnings">
            {#each warnings as w (w.rule + w.text)}<li>{w.text}.</li>{/each}
          </ul>
          {#snippet action()}<Checkbox label="I understand" bind:checked={understood} disabled={busy} />{/snippet}
        </Callout>
      {/if}
    {/if}
    {#snippet footer()}
      <Button variant={crossed ? "primary" : "secondary"} disabled={busy} onclick={() => (confirmOpen = false)}>Cancel</Button>
      <Button
        variant={crossed ? "danger" : "primary"}
        disabled={busy || p.preview.status !== "ready" || breaks || (warnings.length > 0 && !understood)}
        aria-busy={busy}
        onclick={apply}
      >
        {crossed ? "Override for this profile" : "Move it"}
      </Button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .order-view {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .summary {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .rows {
    margin: 0;
    padding: 0;
    list-style: none;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
    min-height: var(--qm-row-h);
    padding: 0 var(--qm-space-2);
    border-bottom: 1px solid var(--qm-border);
    border-left: 4px solid var(--attr, var(--qm-border-strong));
    background: var(--qm-panel);
  }

  .row:focus-visible {
    outline: none;
    box-shadow: inset var(--qm-focus-ring);
  }

  .handle {
    display: inline-flex;
    color: var(--qm-text-muted);
    cursor: grab;
  }

  .n {
    min-width: 3ch;
    color: var(--qm-text-muted);
    font-family: var(--qm-font-mono);
  }

  .name {
    font-weight: 600;
  }

  .mod,
  .prio {
    font-size: var(--qm-text-12);
  }

  .warnings {
    margin: 0;
    padding: 0 0 0 var(--qm-space-4);
  }

  .lock {
    display: inline-flex;
    color: var(--qm-text-muted);
  }

  /* Inferred edges: one dashed lane per edge, from the middle of one row to the middle of the other. */
  .lanes {
    display: flex;
    align-self: stretch;
  }

  .lane {
    position: relative;
    width: 12px;
  }

  .lane::before {
    content: "";
    position: absolute;
    left: 5px;
    width: 2px;
    background: repeating-linear-gradient(to bottom, var(--qm-text-muted) 0 3px, transparent 3px 6px);
  }

  .lane.none::before {
    display: none;
  }

  .lane.through::before {
    top: 0;
    bottom: -1px;
  }

  .lane.top::before {
    top: 50%;
    bottom: -1px;
  }

  .lane.bottom::before {
    top: 0;
    bottom: 50%;
  }

  .dot {
    position: absolute;
    top: calc(50% - 4px);
    left: 2px;
    width: 8px;
    height: 8px;
    border: 2px solid var(--qm-text-muted);
    border-radius: 50%;
    background: var(--qm-panel);
  }

  .c1 { --attr: var(--qm-attr-1); }
  .c2 { --attr: var(--qm-attr-2); }
  .c3 { --attr: var(--qm-attr-3); }
  .c4 { --attr: var(--qm-attr-4); }
  .c5 { --attr: var(--qm-attr-5); }
  .c6 { --attr: var(--qm-attr-6); }
  .c7 { --attr: var(--qm-attr-7); }
  .c8 { --attr: var(--qm-attr-8); }
</style>
