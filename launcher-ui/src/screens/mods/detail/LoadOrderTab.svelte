<script lang="ts">
  import { Badge, Skeleton, StencilHeading } from "../../../components/ui";
  import type { DiagnosticSeverity, ModManifest } from "../../../lib/bindings";
  import { loadOrder } from "../../../lib/stores/loadorder.svelte";
  import { mods } from "../../../lib/stores/mods.svelte";
  import { patches } from "../../../lib/stores/patches.svelte";
  import { openPatchOrder } from "../mods-view.svelte";

  interface Props {
    modId: string;
    enabled: boolean;
    manifest: ModManifest;
  }

  let { modId, enabled, manifest }: Props = $props();

  const TONE: Record<DiagnosticSeverity, "danger" | "warn" | "neutral"> = {
    error: "danger",
    warning: "warn",
    info: "neutral",
  };
  const SEVERITY: Record<DiagnosticSeverity, string> = { error: "Error", warning: "Warning", info: "Info" };

  const patchEntry = $derived(patches.get(modId));
  const touched = $derived(
    patchEntry.status === "ready" ? [...new Set(patchEntry.data.map((p) => p.target_file).filter((f) => f !== ""))].sort() : [],
  );
  const resolution = $derived(loadOrder.resolution.status === "ready" ? loadOrder.resolution.data : null);
  const entry = $derived(resolution?.ordered_mods.find((m) => m.mod_id === modId) ?? null);
  const diagnostics = $derived(
    resolution?.diagnostics.filter((d) => d.mod_id === modId || d.related_mod_id === modId) ?? [],
  );

  /** Installed mods by id; a mod that is not installed shows its id. */
  const names = $derived(Object.fromEntries((mods.list.status === "ready" ? mods.list.data : []).map((m) => [m.id, m.name])));
  const named = (id: string) => names[id] ?? id;

  /** ">=1.0.0" reads as "1.0.0 or newer"; any other range is shown as written. */
  const rangeText = (range: string) => {
    const m = /^>=\s*(\d[\w.+-]*)$/.exec(range.trim());
    return m ? `${m[1]} or newer` : range;
  };
  const ranged = (map: Partial<Record<string, string>> | undefined) =>
    Object.entries(map ?? {}).map(([id, range]) => (range && range !== "*" ? `${named(id)} (${rangeText(range)})` : named(id)));

  const relationships = $derived.by(() => {
    const r = manifest.relationships ?? {};
    return [
      { label: "Requires", items: ranged(r.requires) },
      { label: "Recommends", items: ranged(r.recommends) },
      { label: "Suggests", items: ranged(r.suggests) },
      { label: "Conflicts with", items: ranged(r.conflicts) },
      { label: "Loads after", items: (r.load_after ?? []).map(named) },
      { label: "Loads before", items: (r.load_before ?? []).map(named) },
      { label: "Provides", items: r.provides ?? [] },
    ].filter((g) => g.items.length > 0);
  });
</script>

<div class="load-order">
  <section>
    <StencilHeading level={3}>Position</StencilHeading>
    {#if !resolution}
      <Skeleton lines={2} />
    {:else if entry}
      <p class="position">Loads at position {entry.order_index + 1}</p>
      {#if entry.reasons.length === 1}
        <p class="qm-muted">{entry.reasons[0]}</p>
      {:else if entry.reasons.length > 1}
        <ul class="list">
          {#each entry.reasons as reason, i (i)}
            <li>{reason}</li>
          {/each}
        </ul>
      {/if}
    {:else}
      <p class="qm-muted">{enabled ? "Not in the resolved load order." : "Disabled mods are not loaded."}</p>
    {/if}
  </section>

  <section>
    <StencilHeading level={3}>Relationships</StencilHeading>
    {#if relationships.length === 0}
      <p class="qm-muted">No relationships declared.</p>
    {:else}
      <dl class="relations">
        {#each relationships as group (group.label)}
          <dt>{group.label}</dt>
          <dd>{group.items.join(", ")}</dd>
        {/each}
      </dl>
    {/if}
  </section>

  {#if touched.length > 0}
    <section>
      <StencilHeading level={3}>Patch order</StencilHeading>
      <p class="qm-muted">Where this mod's patches run in each file it changes.</p>
      <ul class="list">
        {#each touched as f (f)}
          <li><button type="button" class="file-link" onclick={() => openPatchOrder(f)}>{f}</button></li>
        {/each}
      </ul>
    </section>
  {/if}

  {#if diagnostics.length > 0}
    <section>
      <StencilHeading level={3}>Diagnostics</StencilHeading>
      <ul class="list diagnostics">
        {#each diagnostics as d, i (i)}
          <li>
            <Badge tone={TONE[d.severity]}>{SEVERITY[d.severity]}</Badge>
            <span>{d.message}</span>
          </li>
        {/each}
      </ul>
    </section>
  {/if}
</div>

<style>
  .file-link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--qm-accent-text);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    text-decoration: underline;
    cursor: pointer;
  }

  .file-link:focus-visible {
    outline: none;
    box-shadow: var(--qm-focus-ring);
  }

  /* The sections sit side by side when the panel is wide. */
  .load-order {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, calc(var(--qm-text-13) * 26)), 1fr));
    gap: var(--qm-space-4) var(--qm-space-6);
    align-items: start;
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  p {
    margin: 0;
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    margin: 0;
    padding-left: var(--qm-space-5);
  }

  .diagnostics {
    padding-left: 0;
    list-style: none;
  }

  .diagnostics li {
    display: flex;
    align-items: baseline;
    gap: var(--qm-space-2);
  }

  .relations {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--qm-space-1) var(--qm-space-3);
    margin: 0;
  }

  .relations dt {
    color: var(--qm-text-muted);
  }

  .relations dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
</style>
