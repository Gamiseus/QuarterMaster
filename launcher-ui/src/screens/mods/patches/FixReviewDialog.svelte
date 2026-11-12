<script lang="ts">
  import { Button, Dialog, RadioGroup } from "../../../components/ui";
  import type { FixView } from "../../../lib/bindings";
  import { patchText, reviewActions, snapshotParts } from "../../../lib/patches/fixlist";
  import { fixes } from "../../../lib/stores/fixes.svelte";

  interface Props {
    open?: boolean;
    fix: FixView | null;
    canMutate: boolean;
    /** The user chose to undo the fix: the caller previews it first. */
    onundo: (fix: FixView) => void;
    /** The user chose to pick a new line for an Anchor fix that uses its own line (§11.4). */
    onrepick?: (fix: FixView) => void;
  }

  let { open = $bindable(false), fix, canMutate, onundo, onrepick }: Props = $props();

  let busy = $state(false);
  /** The patch picked among the fix's candidates (§9.3). */
  let pick = $state<string | undefined>(undefined);
  $effect.pre(() => {
    void fix?.id;
    pick = undefined;
  });

  const actions = $derived(fix ? reviewActions(fix) : { keep: "Keep this fix", undo: "Remove fix", then: "accept" as const });
  const candidates = $derived(
    (fix?.candidates ?? []).map((c) => ({ value: c.patch_id, label: patchText(c), description: c.name && c.name !== c.patch_id ? c.patch_id : undefined })),
  );
  /** Game files the fix's patches target that changed since it was made or accepted (D74). */
  const gameDrift = $derived(fix?.game_drift ?? []);
  /** Patch labels by id, for the diff headings. */
  const labelOf = (id: string) => {
    for (const l of [fix?.patch, ...(fix?.related ?? [])]) if (l && l.patch_id === id) return patchText(l);
    return id;
  };

  async function keep() {
    if (!fix) return;
    if (actions.then === "repick") {
      const f = fix;
      open = false;
      onrepick?.(f);
      return;
    }
    if (actions.then === "rebind" && !pick) return;
    busy = true;
    try {
      if (actions.then === "rebind" && pick) await fixes.rebind(fix.id, pick);
      else await fixes.accept(fix.id);
      open = false;
    } catch {
      // call() already reported it.
    } finally {
      busy = false;
    }
  }

  function undo() {
    if (!fix) return;
    const f = fix;
    open = false;
    onundo(f);
  }
</script>

<Dialog bind:open title={fix ? `Review "${fix.label}"` : "Review fix"} description={fix?.status_text} size="lg" initialFocus="content" dismissible={!busy}>
  {#if fix}
    <div class="review">
      {#if gameDrift.length > 0}
        <section class="drift" aria-label="Game files that changed">
          <h4>The game changed since you made this fix</h4>
          <ul class="files">
            {#each gameDrift as g (g.file)}
              <li><code>{g.file}</code> {g.after ? "changed" : "is no longer in the game"}</li>
            {/each}
          </ul>
          <p class="qm-muted">Check that the fix still does what you want after the game update.</p>
        </section>
      {/if}
      {#each fix.drift ?? [] as d (d.patch_id)}
        <section class="drift" aria-label={`Changes to ${labelOf(d.patch_id)}`}>
          <h4>{labelOf(d.patch_id)}{d.role === "anchor_target" ? " (the patch this fix places code next to)" : ""}</h4>
          <p class="qm-muted">
            When you made this fix{d.before.mod_version ? ` (version ${d.before.mod_version})` : ""}, and now{d.after.mod_version ? ` (version ${d.after.mod_version})` : ""}.
          </p>
          {#each snapshotParts(d.before, d.after) as part (part.label)}
            <p class="part">{part.label}</p>
            <ol class="diff" aria-label={part.label}>
              {#each part.rows as row, i (i)}
                <li class={row.kind}>
                  <span class="sign" aria-hidden="true">{row.kind === "added" ? "+" : row.kind === "removed" ? "-" : ""}</span>
                  <span class="qm-visually-hidden">{row.kind === "added" ? "Now: " : row.kind === "removed" ? "Was: " : ""}</span>
                  <code>{row.text}</code>
                </li>
              {/each}
            </ol>
          {/each}
          {#if d.before.truncated || d.after.truncated}
            <p class="qm-muted">The text is long, so only its start is shown.</p>
          {/if}
        </section>
      {:else}
        {#if gameDrift.length === 0}<p>{fix.status_text}</p>{/if}
      {/each}
      {#if candidates.length > 0}
        <RadioGroup label="Which patch is this fix for?" items={candidates} bind:value={pick} />
      {/if}
    </div>
  {/if}
  {#snippet footer()}
    <Button disabled={busy} onclick={() => (open = false)}>Cancel</Button>
    <Button disabled={busy || !canMutate} onclick={undo}>{actions.undo}...</Button>
    <Button
      variant="primary"
      disabled={busy || !canMutate || (actions.then === "rebind" && !pick) || (actions.then === "repick" && !onrepick)}
      aria-busy={busy}
      onclick={keep}>{actions.keep}</Button
    >
  {/snippet}
</Dialog>

<style>
  .review {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-4);
  }

  .drift {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  h4,
  p {
    margin: 0;
  }

  h4 {
    font-size: var(--qm-text-14);
  }

  .files {
    margin: 0;
    padding-left: var(--qm-space-4);
  }

  .files code {
    font-family: var(--qm-font-mono);
  }

  .part {
    font-weight: 600;
    font-size: var(--qm-text-13);
  }

  .diff {
    margin: 0;
    padding: var(--qm-space-2) 0;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-raised);
    list-style: none;
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  .diff li {
    display: flex;
    gap: var(--qm-space-2);
    padding: 0 var(--qm-space-3);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .diff li.added {
    background: color-mix(in srgb, var(--qm-ok) 14%, transparent);
  }

  .diff li.removed {
    background: color-mix(in srgb, var(--qm-danger) 14%, transparent);
  }

  .sign {
    flex: none;
    width: 1ch;
    color: var(--qm-text-muted);
  }

  code {
    font: inherit;
  }
</style>
