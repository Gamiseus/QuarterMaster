<script lang="ts">
  import type { SolverDiagnostic } from "../lib/bindings";
  import { fixFor, type FixAction } from "../lib/modlist/fixes";
  import { app } from "../lib/stores/app.svelte";
  import { game } from "../lib/stores/game.svelte";
  import { mods } from "../lib/stores/mods.svelte";
  import { updates } from "../lib/stores/updates.svelte";
  import { runFix } from "../screens/mods/run-fix.svelte";
  import { Button } from "./ui";

  interface Props {
    diagnostic: SolverDiagnostic;
    /** Runs after the fix went through, e.g. to close a dialog when the fix opened another view. */
    onfixed?: (action: FixAction) => void;
  }

  let { diagnostic, onfixed }: Props = $props();

  let running = $state(false);

  const fix = $derived(
    fixFor(diagnostic, mods.list.status === "ready" ? mods.list.data : [], updates.report.status === "ready" ? updates.report.data : null),
  );

  async function run() {
    if (!fix || running) return;
    const action = fix.action;
    running = true;
    try {
      await runFix(action);
      onfixed?.(action);
    } catch {
      // The change was refused; call() already showed the banner or toast.
    } finally {
      running = false;
    }
  }
</script>

{#if fix}
  <Button size="sm" disabled={!app.startupDone || game.isBusy || running} onclick={run}>{fix.label}</Button>
{/if}
