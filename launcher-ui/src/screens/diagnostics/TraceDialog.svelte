<script lang="ts">
  import { Button, Callout, Dialog, Skeleton, TextField } from "../../components/ui";
  import { call } from "../../lib/api";
  import { commands, type DiagnosticRunResult } from "../../lib/bindings";
  import {
    callerLine,
    changeLine,
    globalTraces,
    hookSummary,
    parseNames,
    resultStatus,
    suggestedText,
    traceSummary,
  } from "../../lib/diagnostics/trace";
  import { app } from "../../lib/stores/app.svelte";
  import { game } from "../../lib/stores/game.svelte";
  import { mods } from "../../lib/stores/mods.svelte";
  import { profiles } from "../../lib/stores/profiles.svelte";

  interface Props {
    open: boolean;
  }

  let { open = $bindable(false) }: Props = $props();

  let globals = $state("");
  let hooks = $state("");
  let loaded = $state(false);
  let result = $state<DiagnosticRunResult | null>(null);
  let busy = $state(false);
  let seq = 0;

  const modName = (id: string) => (mods.list.status === "ready" ? mods.list.data.find((m) => m.id === id)?.name : undefined) ?? id;
  const blocked = $derived(
    !app.startupDone
      ? "Available once startup has finished."
      : game.isBusy
        ? "Close the game to start a test run."
        : (profiles.activeSummary?.is_builtin ?? false)
          ? "The Vanilla profile loads no mods, so there is nothing to trace."
          : null,
  );
  const nothing = $derived(parseNames(globals).length + parseNames(hooks).length === 0);

  async function loadResult() {
    const mine = ++seq;
    try {
      const r = await call(commands.diagnosticRunResult());
      if (mine === seq) result = r;
    } catch {
      // call() showed the error.
    }
  }

  // Each time the dialog opens: the suggested names (once) and the newest result.
  $effect(() => {
    if (!open) return;
    void loadResult();
    if (loaded) return;
    call(commands.diagnosticRunPreview())
      .then((p) => {
        const s = suggestedText(p);
        if (!globals) globals = s.globals;
        if (!hooks) hooks = s.hooks;
      })
      .catch(() => {})
      .finally(() => (loaded = true));
  });

  // The result is complete once the game has closed.
  $effect(() => {
    if (open && !game.isBusy) void loadResult();
  });

  async function start() {
    if (busy || blocked || nothing) return;
    busy = true;
    try {
      await call(commands.gameLaunchDiagnostic({ globals: parseNames(globals), hooks: parseNames(hooks) }));
      await loadResult();
    } catch {
      // call() showed the error.
    } finally {
      busy = false;
    }
  }
</script>

<Dialog
  bind:open
  title="Trace globals in a test run"
  description="Starts the game once and notes, while it loads, which mod sets each name below and in what order. It also counts the places that call the functions you list. Normal launches are never traced."
  initialFocus="content"
  dismissible={!busy}
>
  {#if !loaded}
    <div aria-busy="true" aria-label="Loading the suggested names"><Skeleton lines={2} /></div>
  {:else}
    <TextField label="Globals to watch" bind:value={globals} placeholder="loadModSettings, saveModSettings" disabled={busy} />
    <TextField label="Functions whose callers to count" bind:value={hooks} placeholder="onDeath, Events.fire" disabled={busy} />
    <p class="qm-muted">
      Separate names with commas. The fields start with the names the code checks flagged. Your save is backed up
      first, and the game loads more slowly during this run.
    </p>
  {/if}
  {#if blocked}<p class="qm-muted" role="note">{blocked}</p>{/if}

  {#if result}
    {@const r = result}
    <section class="result" aria-label="Last diagnostic run" data-testid="trace-result">
      <h4>Last run</h4>
      <p class="qm-muted">{resultStatus(r)}</p>
      {#if r.reported || r.globals_trace.length > 0}
        {#each globalTraces(r) as t (t.name)}
          <div class="item" data-testid="trace-global">
            <p><span class="name">{t.name}</span> {traceSummary(t)}</p>
            {#if t.changes.length > 0}
              <ol>
                {#each t.changes as c (c.seq)}<li>{changeLine(c, modName)}</li>{/each}
              </ol>
            {/if}
            {#if t.mods > 1}
              <Callout tone="warn" title="More than one mod sets this name.">
                The last one in the list is the one the game ends up with. An order rule under Patches can change which
                mod runs last.
              </Callout>
            {/if}
          </div>
        {/each}
        {#each r.hook_calls as h (h.name)}
          <div class="item" data-testid="trace-hook">
            <p><span class="name">{h.name}</span> {hookSummary(h)}</p>
            {#if (h.callers ?? []).length > 0}
              <ul>
                {#each h.callers ?? [] as c (c.source + ":" + c.line)}<li>{callerLine(c, modName)}</li>{/each}
              </ul>
            {/if}
          </div>
        {/each}
      {/if}
    </section>
  {/if}

  {#snippet footer()}
    <Button disabled={busy} onclick={() => (open = false)}>Close</Button>
    <Button variant="primary" disabled={busy || blocked !== null || nothing || !loaded} aria-busy={busy} onclick={start}>
      Start test run
    </Button>
  {/snippet}
</Dialog>

<style>
  p,
  h4 {
    margin: 0;
  }

  p,
  li {
    font-size: var(--qm-text-12);
  }

  h4 {
    font-size: var(--qm-text-13);
    font-weight: 600;
  }

  .result {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    padding-top: var(--qm-space-3);
    border-top: 1px solid var(--qm-border);
  }

  .item {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  .name {
    font-family: var(--qm-font-mono);
    font-weight: 600;
  }

  ol,
  ul {
    margin: 0;
    padding-left: var(--qm-space-5);
  }
</style>
