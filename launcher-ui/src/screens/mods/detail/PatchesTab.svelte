<script lang="ts">
  import { Badge, Button, Dialog, Skeleton, StencilHeading, Switch } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import {
    commands,
    type ConflictResolutionSuggestion,
    type DetectedConflict,
    type ModPatchStatus,
    type PatchState,
  } from "../../../lib/bindings";
  import { patches } from "../../../lib/stores/patches.svelte";

  interface Props {
    modId: string;
    canMutate: boolean;
  }

  let { modId, canMutate }: Props = $props();

  type Winner = Extract<ConflictResolutionSuggestion, { tier: "select_winner" }>;

  const STATE: Record<PatchState, { label: string; tone: "ok" | "danger" | "neutral" }> = {
    applied: { label: "Applied", tone: "ok" },
    broken: { label: "Broken", tone: "danger" },
    disabled: { label: "Disabled", tone: "neutral" },
    skipped: { label: "Skipped", tone: "neutral" },
  };
  const SUGGESTION: Record<ConflictResolutionSuggestion["tier"], string> = {
    swap_mod_order: "Swap order",
    generate_glue_patch: "Use glue patch",
    select_winner: "Pick winner",
  };

  const entry = $derived(patches.get(modId));
  const list = $derived(entry.status === "ready" ? entry.data : []);
  const conflicts = $derived.by(() => {
    const byId = new Map<string, DetectedConflict>();
    for (const p of list) for (const c of p.conflicts) byId.set(c.id, c);
    return [...byId.values()];
  });

  /** Toggles in flight; dropped when fresh patch data arrives or the write fails. */
  let overrides = $state<Record<string, boolean>>({});
  let busy = $state(false);
  /** Patch ids whose toggle is still being written. */
  let pending = $state<Set<string>>(new Set());
  let picking = $state<Winner | null>(null);
  let pickOpen = $state(false);

  $effect(() => {
    void entry;
    overrides = {};
  });

  const nameOf = (p: ModPatchStatus) => p.name ?? p.patch_id;
  const patchName = (id: string) => {
    const p = list.find((x) => x.patch_id === id);
    return p ? nameOf(p) : id;
  };

  async function toggle(p: ModPatchStatus, enabled: boolean) {
    const id = p.patch_id;
    overrides = { ...overrides, [id]: enabled };
    pending = new Set([...pending, id]);
    try {
      await call(commands.patchSetEnabled(id, enabled));
    } catch {
      const next = { ...overrides };
      delete next[id];
      overrides = next;
    } finally {
      const next = new Set(pending);
      next.delete(id);
      pending = next;
    }
  }

  async function resolve(s: ConflictResolutionSuggestion, winner: number | null) {
    busy = true;
    try {
      await call(commands.conflictResolve(s, winner));
    } catch {
      // call() already showed the error.
    } finally {
      busy = false;
    }
  }

  function suggest(s: ConflictResolutionSuggestion) {
    if (s.tier === "select_winner") {
      picking = s;
      pickOpen = true;
    } else {
      void resolve(s, null);
    }
  }

  function pick(index: 0 | 1) {
    const s = picking;
    pickOpen = false;
    if (s) void resolve(s, index);
  }
</script>

<div class="patches">
  {#if entry.status === "loading"}
    <div aria-busy="true" aria-label="Loading patches"><Skeleton lines={3} /></div>
  {:else if entry.status === "error"}
    <div class="error" role="alert">
      <p>{entry.message}</p>
      <Button size="sm" onclick={() => patches.load(modId).catch(() => {})}>Retry</Button>
    </div>
  {:else if list.length === 0}
    <p class="qm-muted">This mod has no patches.</p>
  {:else}
    <ul class="rows">
      {#each list as p (p.patch_id)}
        <li class="row">
          <div class="main">
            <span class="name">{nameOf(p)}</span>
            <code class="file">{p.target_file}</code>
          </div>
          <Badge tone={STATE[p.state].tone}>{STATE[p.state].label}</Badge>
          <Switch
            checked={overrides[p.patch_id] ?? p.state !== "disabled"}
            label={`Enable ${nameOf(p)}`}
            hideLabel
            disabled={!canMutate || pending.has(p.patch_id)}
            onCheckedChange={(v: boolean) => toggle(p, v)}
          />
        </li>
      {/each}
    </ul>
  {/if}

  {#if conflicts.length > 0}
    <section class="conflicts">
      <StencilHeading level={3}>Conflicts</StencilHeading>
      {#each conflicts as c (c.id)}
        <div class="conflict">
          <p>{c.description}</p>
          <code class="file">{c.target_file}</code>
          {#if c.suggestions.length > 0}
            <div class="actions">
              {#each c.suggestions as s, i (i)}
                <Button size="sm" disabled={!canMutate || busy} title={s.explanation} onclick={() => suggest(s)}>
                  {SUGGESTION[s.tier]}
                </Button>
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    </section>
  {/if}
</div>

<Dialog
  bind:open={pickOpen}
  title="Pick winner"
  initialFocus="content"
  description="Choose which patch to keep. The other one is disabled."
>
  {#if picking}
    <div class="choices">
      <Button onclick={() => pick(0)}>Keep {patchName(picking.patch_a_id)}</Button>
      <Button onclick={() => pick(1)}>Keep {patchName(picking.patch_b_id)}</Button>
    </div>
  {/if}
  {#snippet footer()}
    <Button variant="ghost" onclick={() => (pickOpen = false)}>Cancel</Button>
  {/snippet}
</Dialog>

<style>
  .patches {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-4);
  }

  p {
    margin: 0;
  }

  .error {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--qm-space-2);
  }

  .rows {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .main {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .name {
    overflow-wrap: anywhere;
  }

  .file {
    color: var(--qm-text-muted);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-11);
    overflow-wrap: anywhere;
  }

  .conflicts {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .conflict {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    padding: var(--qm-space-2) var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-raised);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-2);
    margin-top: var(--qm-space-1);
  }

  .choices {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }
</style>
