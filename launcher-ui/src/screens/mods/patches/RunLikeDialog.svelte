<script lang="ts">
  import { ConfirmDialog, Skeleton } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type FixView, type PreviewResult, type RunAs } from "../../../lib/bindings";
  import { couldNotCheck, patchText, runLikeDraft } from "../../../lib/patches/fixlist";
  import { fixes } from "../../../lib/stores/fixes.svelte";
  import ImpactNotes from "./ImpactNotes.svelte";

  interface Props {
    open?: boolean;
    /** An older adjusted copy that runs after every mod's patches, or one switched to its mod (D58). */
    fix: FixView | null;
    modNames: Record<string, string>;
    /** `base` runs the copy like its mod; `legacy` runs it last again. */
    runAs?: RunAs;
  }

  let { open = $bindable(false), fix, modNames, runAs = "base" }: Props = $props();

  let preview = $state<PreviewResult | null>(null);
  let error = $state<string | null>(null);
  let token = 0;

  const mod = $derived(fix?.run_like_mod ? (modNames[fix.run_like_mod] ?? fix.run_like_mod) : "its mod");
  const base = $derived(fix?.patch ? patchText(fix.patch) : "the patch it stands in for");
  const name = (patchId: string) => {
    for (const f of fixes.list) {
      for (const l of [f.patch, ...f.related]) if (l && l.patch_id === patchId) return l.name ?? patchId;
    }
    return patchId;
  };

  // Each opening previews the change again; the profile may have changed since.
  $effect(() => {
    if (!open || !fix) return;
    const mine = ++token;
    const draft = runLikeDraft(fix, runAs);
    preview = null;
    error = null;
    if (!draft) {
      error = "This copy cannot change how it runs.";
      return;
    }
    call(commands.simulationPreview({ add_fixes: [draft], focus_file: fix.target_file ?? null }), { expect: ["cancelled", "conflict", "invalid_input", "not_found"] })
      .then((r) => {
        if (mine === token) preview = r;
      })
      .catch((e: unknown) => {
        if (mine === token) error = couldNotCheck(e);
      });
  });

  const blocked = $derived(preview === null || preview.impact.breaks_file);

  async function confirm() {
    const draft = fix ? runLikeDraft(fix, runAs) : null;
    if (!draft) return;
    await fixes.create(draft);
  }

  const last = $derived(runAs === "legacy");
  const title = $derived(
    !fix ? (last ? "Run the copy last again?" : "Run the copy like its mod?")
    : last ? `Run "${fix.label}" last again?` : `Run "${fix.label}" like ${mod}?`,
  );
</script>

<ConfirmDialog
  bind:open
  {title}
  body={last
    ? `The copy runs after every mod's patches again, instead of in the place of ${base}.`
    : `The copy runs as ${mod}, in the place of ${base}, instead of after every mod's patches.`}
  confirmLabel={last ? "Run it last again" : `Run it like ${mod}`}
  danger={false}
  confirmDisabled={blocked}
  onconfirm={confirm}
>
  <div class="run-like" aria-live="polite">
    {#if error}
      <p class="error" role="alert">{error}</p>
    {:else if !preview}
      <div aria-busy="true" aria-label="Checking what changes"><Skeleton lines={2} /></div>
    {:else}
      <ImpactNotes impact={preview.impact} {modNames} {name} />
    {/if}
  </div>
</ConfirmDialog>

<style>
  .run-like {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  p {
    margin: 0;
  }

  .error {
    color: var(--qm-danger);
  }
</style>
