<script lang="ts">
  import { untrack } from "svelte";
  import { Badge, Button, Callout, Switch } from "../../components/ui";
  import { call } from "../../lib/api";
  import { commands } from "../../lib/bindings";
  import { app } from "../../lib/stores/app.svelte";
  import { loadOrder } from "../../lib/stores/loadorder.svelte";
  import { isProblemLine, linkOf } from "../../lib/engine/testlog";
  import { engine } from "../../lib/stores/engine.svelte";
  import { game } from "../../lib/stores/game.svelte";
  import { logs } from "../../lib/stores/logs.svelte";
  import { saves } from "../../lib/stores/saves.svelte";
  import { studio } from "../../lib/stores/studio.svelte";

  interface Props {
    onclose: () => void;
  }

  let { onclose }: Props = $props();

  let all = $state(false);

  $effect(() => {
    logs.ensureLoaded("runtime");
    logs.ensureLoaded("runtime_errors");
  });

  const files = $derived(engine.tree.filter((e) => !e.folder).map((e) => e.path));
  /** Module patches load the mod's file under their own name; the log uses that name. */
  const modules = $derived(
    new Map(
      (studio.patches?.patches ?? [])
        .filter((p) => p.draft.kind === "module" && p.draft.name && p.draft.source[0])
        .map((p) => [p.draft.name, p.draft.source[0]] as [string, string]),
    ),
  );
  const lines = $derived.by(() => {
    const merged = [...logs.lines.runtime_errors, ...logs.lines.runtime].sort((a, b) => a.seq - b.seq);
    const shown = all ? merged : merged.filter((l) => isProblemLine(l.line));
    return shown.slice(-300).map((l) => ({ ...l, link: linkOf(l.line, files, modules) }));
  });

  const stage = $derived(game.status.stage.stage);
  const STAGE: Record<string, { tone: "ok" | "warn" | "danger" | "info"; text: string }> = {
    idle: { tone: "ok", text: "Not running" },
    deploying: { tone: "info", text: "Getting the game ready" },
    running: { tone: "info", text: "The game is running" },
    restoring: { tone: "info", text: "Putting the game folder back" },
    failed: { tone: "danger", text: "The launch failed" },
  };

  const plan = $derived(engine.testPlan);
  const summary = $derived.by(() => {
    if (!plan) return "";
    const req = plan.requires.length;
    const off = plan.turned_off.length;
    const needs = req ? ` with ${req} required ${req === 1 ? "mod" : "mods"}` : "";
    const others = `${off} other ${off === 1 ? "mod is" : "mods are"} off for the run`;
    return `Runs ${plan.mod_name}${needs}; ${others}. Your save slot is backed up first.`;
  });

  /** The save backup taken before the last test, while it can be put back. */
  let backup = $state<{ slot: string; id: string } | null>(null);
  let restoreAsked = $state(false);
  let restoring = $state(false);
  let restored = $state(false);
  let ran = false;

  // When a test ends, find the save backup it made.
  $effect(() => {
    if (stage !== "idle") {
      ran = true;
      return;
    }
    if (!ran) return;
    ran = false;
    untrack(() => void findBackup());
  });

  async function findBackup() {
    const profileId = app.settings?.active_profile_id;
    if (!profileId) return;
    const runs = await call(commands.runsList(profileId)).catch(() => []);
    const tests = runs.filter((r) => r.trial?.label.startsWith("Test of ") && r.trial.save_slot_id && r.trial.save_backup_id);
    const last = tests.sort((a, b) => b.started_at.localeCompare(a.started_at))[0]?.trial;
    backup = last?.save_slot_id && last.save_backup_id ? { slot: last.save_slot_id, id: last.save_backup_id } : null;
    restored = false;
    restoreAsked = false;
  }

  async function putBack() {
    if (!backup) return;
    restoring = true;
    try {
      await saves.restore(backup.slot, backup.id);
      restored = true;
      restoreAsked = false;
    } catch {
      // call() reported it.
    } finally {
      restoring = false;
    }
  }

  async function turnOn(id: string) {
    await loadOrder.whileChanging(() => call(commands.modSetEnabled(id, true))).catch(() => {});
    await engine.loadTestPlan().catch(() => {});
  }

  async function sync() {
    await app.syncWorkspace().catch(() => {});
    await engine.load().catch(() => {});
    await engine.loadTestPlan().catch(() => {});
  }

  async function open(path: string, line: number) {
    engine.view = "files";
    await engine.open(path).catch(() => {});
    engine.jumpTo(path, line);
  }
</script>

<section class="test" aria-label="Test run">
  <header class="head">
    <h2 class="qm-stencil title">Test run</h2>
    <Badge tone={STAGE[stage]?.tone ?? "info"}>{STAGE[stage]?.text ?? stage}</Badge>
    <span class="spacer"></span>
    <Switch label="Every line" checked={all} onCheckedChange={(v: boolean) => (all = v)} />
    <Button size="sm" variant="ghost" onclick={onclose}>Hide</Button>
  </header>
  {#if plan?.blocked}
    <Callout tone="warn" title="This test cannot run yet">
      {plan.message}
      {#snippet action()}
        {#if plan.blocked === "off"}
          <Button size="sm" onclick={() => turnOn(plan.mod_id)}>Turn it on</Button>
        {:else if plan.blocked === "required_off"}
          {@const off = plan.requires.find((r) => !r.enabled)}
          {#if off}<Button size="sm" onclick={() => turnOn(off.id)}>Turn on {off.name}</Button>{/if}
        {:else if plan.blocked === "not_in_set"}
          <Button size="sm" onclick={sync}>Sync now</Button>
        {/if}
      {/snippet}
    </Callout>
  {:else if plan}
    <p class="qm-muted summary">
      {summary}
      {app.settings?.debug_mode ? "Debug mode is on." : "Debug mode is off; turn it on in Settings for the console window and the patched files."}
      {#if !app.settings?.debug_mode}<button type="button" class="link" onclick={() => app.goTo("launcher", "settings")}>Settings</button>{/if}
    </p>
  {/if}
  {#if backup && stage === "idle"}
    <div class="restore">
      {#if restored}
        <span class="qm-muted">The save is back to how it was before the test.</span>
      {:else if restoreAsked}
        <span>The save goes back to how it was before the test. Progress made in the test is lost.</span>
        <Button size="sm" variant="danger" disabled={restoring} onclick={putBack}>Put the save back</Button>
        <Button size="sm" variant="ghost" disabled={restoring} onclick={() => (restoreAsked = false)}>Keep it</Button>
      {:else}
        <span class="qm-muted">The save was backed up before the test.</span>
        <Button size="sm" variant="ghost" onclick={() => (restoreAsked = true)}>Put the save back</Button>
      {/if}
    </div>
  {/if}
  {#if lines.length === 0}
    <p class="qm-muted empty">{all ? "The game has written nothing yet." : "No errors or warnings so far."}</p>
  {:else}
    <ol class="log" aria-label="Log lines">
      {#each lines as l (l.seq)}
        <li class={["line", { problem: isProblemLine(l.line) }]}>
          {#if l.link}
            {l.line.slice(0, l.link.start)}<button type="button" class="link" onclick={() => l.link && open(l.link.path, l.link.line)}
              >{l.line.slice(l.link.start, l.link.end)}</button
            >{l.line.slice(l.link.end)}
          {:else}
            {l.line}
          {/if}
        </li>
      {/each}
    </ol>
  {/if}
</section>

<style>
  .test {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    gap: var(--qm-space-1);
    min-height: 0;
    padding-top: var(--qm-space-2);
    border-top: 1px solid var(--qm-border);
  }

  .head {
    display: flex;
    align-items: center;
    gap: var(--qm-space-3);
  }

  .title {
    margin: 0;
    font-size: var(--qm-text-13);
    color: var(--qm-text-muted);
  }

  .spacer {
    flex: 1;
  }

  .restore {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
    font-size: var(--qm-text-12);
  }

  .summary {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .empty {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .log {
    position: relative;
    margin: 0;
    padding: 0;
    overflow: auto;
    list-style: none;
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  .line {
    padding: 1px var(--qm-space-2);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .line.problem {
    color: var(--qm-warn);
  }

  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--qm-accent-text);
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
</style>
