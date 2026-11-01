<script lang="ts">
  import { Badge, Skeleton, StencilHeading } from "../../../components/ui";
  import type { DiagnosticSeverity, ModManifest } from "../../../lib/bindings";
  import { loadOrder } from "../../../lib/stores/loadorder.svelte";

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

  const resolution = $derived(loadOrder.resolution.status === "ready" ? loadOrder.resolution.data : null);
  const entry = $derived(resolution?.ordered_mods.find((m) => m.mod_id === modId) ?? null);
  const diagnostics = $derived(
    resolution?.diagnostics.filter((d) => d.mod_id === modId || d.related_mod_id === modId) ?? [],
  );

  const ranged = (map: Partial<Record<string, string>> | undefined) =>
    Object.entries(map ?? {}).map(([id, range]) => (range && range !== "*" ? `${id} ${range}` : id));

  const relationships = $derived.by(() => {
    const r = manifest.relationships ?? {};
    return [
      { label: "Requires", items: ranged(r.requires) },
      { label: "Recommends", items: ranged(r.recommends) },
      { label: "Suggests", items: ranged(r.suggests) },
      { label: "Conflicts with", items: ranged(r.conflicts) },
      { label: "Loads after", items: r.load_after ?? [] },
      { label: "Loads before", items: r.load_before ?? [] },
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
      {#if entry.reasons.length > 0}
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
  .load-order {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-4);
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
