<script lang="ts">
  import { Button, Callout, Dialog, Skeleton } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type PreviewResult, type StackMode } from "../../../lib/bindings";
  import { MODE_TEXT } from "../../../lib/patches/stacks";
  import { stacking } from "../../../lib/stores/stacking.svelte";
  import ImpactNotes from "./ImpactNotes.svelte";

  interface Props {
    open?: boolean;
    /** The mode to preview and apply. */
    mode: StackMode;
    modNames: Record<string, string>;
    canMutate: boolean;
  }

  let { open = $bindable(false), mode, modNames, canMutate }: Props = $props();

  const TITLE: Record<StackMode, string> = {
    dependencies: "Use dependency order?",
    load_order: "Use load order?",
    legacy: "Use the older order?",
  };

  let preview = $state<{ status: "loading" } | { status: "ready"; data: PreviewResult } | { status: "error"; message: string }>({ status: "loading" });
  let busy = $state(false);
  let seq = 0;

  async function load(m: StackMode) {
    const mine = ++seq;
    preview = { status: "loading" };
    try {
      const data = await call(commands.simulationPreview({ stacking: m }), { expect: ["cancelled", "conflict"] });
      if (mine === seq) preview = { status: "ready", data };
    } catch (e) {
      if (mine === seq) preview = { status: "error", message: e instanceof Error ? e.message : String(e) };
    }
  }

  $effect(() => {
    if (open) void load(mode);
  });

  const name = (id: string) => {
    if (preview.status !== "ready") return id;
    for (const c of preview.data.impact.stack_changes ?? []) {
      const b = [...c.before, ...c.after].find((x) => x.patch_id === id);
      if (b) return b.patch_name || id;
    }
    return id;
  };
  const breaks = $derived(preview.status === "ready" && preview.data.impact.breaks_file);

  async function apply() {
    busy = true;
    try {
      await stacking.set(mode);
      open = false;
    } catch {
      // call() showed the error.
    } finally {
      busy = false;
    }
  }
</script>

<Dialog bind:open title={TITLE[mode]} description={MODE_TEXT[mode]} initialFocus="content" size="lg" dismissible={!busy}>
  {#if preview.status === "loading"}
    <div aria-busy="true" aria-label="Checking what changes"><Skeleton lines={4} /></div>
  {:else if preview.status === "error"}
    <Callout tone="danger" title="Could not check what this changes.">
      {preview.message}
      {#snippet action()}<Button size="sm" onclick={() => load(mode)}>Retry</Button>{/snippet}
    </Callout>
  {:else if (preview.data.impact.stack_changes ?? []).length === 0 && !breaks}
    <p class="none">No shared line changes order in this profile.</p>
  {:else}
    <ImpactNotes impact={preview.data.impact} {modNames} {name} />
  {/if}
  {#snippet footer()}
    <Button disabled={busy} onclick={() => (open = false)}>Cancel</Button>
    <Button
      variant="primary"
      disabled={!canMutate || busy || preview.status !== "ready" || breaks}
      aria-busy={busy}
      data-autofocus=""
      onclick={apply}
    >
      Apply
    </Button>
  {/snippet}
</Dialog>

<style>
  .none {
    margin: 0;
  }
</style>
