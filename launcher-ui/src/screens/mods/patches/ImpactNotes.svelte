<script lang="ts">
  import { Callout } from "../../../components/ui";
  import type { PreviewImpact } from "../../../lib/bindings";
  import { outcomeChanges } from "../../../lib/patches/impact";
  import FactionNote from "./FactionNote.svelte";
  import StackChangeList from "./StackChangeList.svelte";

  interface Props {
    impact: PreviewImpact;
    modNames: Record<string, string>;
    name: (patchId: string) => string;
  }

  let { impact, modNames, name }: Props = $props();

  const outcomes = $derived(outcomeChanges(impact, name));
  const stacks = $derived(impact.stack_changes ?? []);
  const nothing = $derived(
    impact.files_changed.length === 0 &&
      outcomes.length === 0 &&
      stacks.length === 0 &&
      impact.compile_errors_fixed.length === 0 &&
      (impact.faction_changes ?? []).length === 0,
  );
</script>

<div class="impact">
  {#if impact.breaks_file}
    <Callout tone="danger" title="This change would stop a game file from compiling.">
      {impact.compile_errors_new.join(", ")} would not compile, so it cannot be applied.
    </Callout>
  {/if}
  <FactionNote {impact} {modNames} />
  {#if nothing}
    <p class="qm-muted">The game code does not change.</p>
  {/if}
  {#if outcomes.length > 0}
    <p>Also: {outcomes.join("; ")}.</p>
  {/if}
  {#if impact.compile_errors_fixed.length > 0}
    <p>Makes {impact.compile_errors_fixed.join(", ")} compile again.</p>
  {/if}
  {#if impact.files_changed.length > 0}
    <p class="qm-muted">Changes {impact.files_changed.length === 1 ? "1 file" : `${impact.files_changed.length} files`}: {impact.files_changed.join(", ")}.</p>
  {/if}
  {#if stacks.length > 0}
    <p>Changes the order at {stacks.length === 1 ? "1 shared line" : `${stacks.length} shared lines`}:</p>
    <StackChangeList changes={stacks} {modNames} />
  {/if}
</div>

<style>
  .impact {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  p {
    margin: 0;
  }
</style>
