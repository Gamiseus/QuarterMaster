<script lang="ts">
  import { untrack } from "svelte";
  import { Button, Dialog, Skeleton, StencilHeading } from "../../components/ui";
  import type { SaveComparison } from "../../lib/bindings";
  import { diffFingerprints } from "../../lib/saves/fingerprint";
  import { metricRows, signed } from "../../lib/saves/format";
  import { saves } from "../../lib/stores/saves.svelte";

  interface Props {
    open?: boolean;
    /** The slot compared from. */
    a: string | null;
    /** The slot compared to; deltas are `b` minus `a`. */
    b: string | null;
  }

  let { open = $bindable(false), a, b }: Props = $props();

  type State = { status: "loading" } | { status: "ready"; data: SaveComparison } | { status: "error"; message: string };
  let result = $state<State>({ status: "loading" });
  let seq = 0;

  async function load(x: string, y: string) {
    const mine = ++seq;
    result = { status: "loading" };
    try {
      const data = await saves.compare(x, y);
      if (mine === seq) result = { status: "ready", data };
    } catch (e) {
      if (mine === seq) result = { status: "error", message: e instanceof Error ? e.message : String(e) };
    }
  }

  $effect(() => {
    if (open && a && b) {
      const [x, y] = [a, b];
      untrack(() => void load(x, y));
    }
  });

  const fingerprint = (id: string | null) => saves.list?.find((s) => s.profile.id === id)?.profile.active_mods_fingerprint ?? {};
  // The mod section uses each slot's fingerprint, not the save's own metadata.
  const mods = $derived(diffFingerprints(fingerprint(a), fingerprint(b)));
  const fpB = $derived(fingerprint(b));
  const fpA = $derived(fingerprint(a));
  const nameA = $derived(a ? saves.nameFor(a) : "");
  const nameB = $derived(b ? saves.nameFor(b) : "");
  const noMods = $derived(mods.added.length + mods.removed.length + mods.changed.length === 0);
</script>

<Dialog bind:open title="{nameA} and {nameB}" size="lg">
  {#if result.status === "loading"}
    <div aria-busy="true" aria-label="Comparing the slots"><Skeleton lines={6} /></div>
  {:else if result.status === "error"}
    <p role="alert">{result.message}</p>
  {:else}
    {@const c = result.data}
    <div class="compare">
      <p class="qm-muted area">{c.ao_a || "No campaign"} to {c.ao_b || "No campaign"}</p>
      <table class="metrics">
        <thead>
          <tr><th scope="col">Figure</th><th scope="col" class="num">{nameA}</th><th scope="col" class="num">{nameB}</th><th scope="col" class="num">Change</th></tr>
        </thead>
        <tbody>
          {#each metricRows(c) as row (row.label)}
            <tr>
              <th scope="row">{row.label}</th>
              <td class="num">{row.a}</td>
              <td class="num">{row.b}</td>
              <td class="num">{signed(row.delta)}</td>
            </tr>
          {/each}
        </tbody>
      </table>

      <StencilHeading level={3}>Soldiers</StencilHeading>
      {#if c.added_soldiers.length + c.lost_soldiers.length + c.promoted_soldiers.length === 0}
        <p class="qm-muted none">No changes to the deployed squad.</p>
      {:else}
        <div class="groups">
          {#each [{ title: `Only in ${nameB}`, items: c.added_soldiers }, { title: `Only in ${nameA}`, items: c.lost_soldiers }, { title: "Promoted", items: c.promoted_soldiers }] as group (group.title)}
            {#if group.items.length > 0}
              <div>
                <p class="group-title">{group.title}</p>
                <ul class="items">
                  {#each group.items as item (item)}<li>{item}</li>{/each}
                </ul>
              </div>
            {/if}
          {/each}
        </div>
      {/if}

      <StencilHeading level={3}>Mods</StencilHeading>
      {#if noMods}
        <p class="qm-muted none">Both slots were last used with the same mods.</p>
      {:else}
        <div class="groups">
          {#if mods.added.length > 0}
            <div>
              <p class="group-title">Only in {nameB}</p>
              <ul class="items">{#each mods.added as id (id)}<li>{id} {fpB[id]}</li>{/each}</ul>
            </div>
          {/if}
          {#if mods.removed.length > 0}
            <div>
              <p class="group-title">Only in {nameA}</p>
              <ul class="items">{#each mods.removed as id (id)}<li>{id} {fpA[id]}</li>{/each}</ul>
            </div>
          {/if}
          {#if mods.changed.length > 0}
            <div>
              <p class="group-title">Different version</p>
              <ul class="items">{#each mods.changed as m (m.id)}<li>{m.id} {m.from} to {m.to}</li>{/each}</ul>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
  {#snippet footer()}
    <Button onclick={() => (open = false)}>Close</Button>
  {/snippet}
</Dialog>

<style>
  .compare {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .area,
  .none {
    margin: 0;
    font-size: var(--qm-text-13);
  }

  .metrics {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--qm-text-13);
  }

  .metrics th,
  .metrics td {
    padding: var(--qm-space-1) var(--qm-space-2);
    border-bottom: 1px solid var(--qm-border);
    text-align: left;
  }

  .metrics thead th {
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
    font-weight: 400;
  }

  .metrics tbody th {
    font-weight: 400;
  }

  .metrics .num {
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .groups {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(calc(var(--qm-detail-w) * 0.6), 1fr));
    gap: var(--qm-space-3);
  }

  .group-title {
    margin: 0 0 var(--qm-space-1);
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }

  .items {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--qm-text-13);
  }
</style>
