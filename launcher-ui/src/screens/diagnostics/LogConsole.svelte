<script lang="ts">
  import { tick, untrack } from "svelte";
  import { Button, Panel, StencilHeading, Tabs, TextField } from "../../components/ui";
  import type { LogSource } from "../../lib/bindings";
  import { isAtBottom } from "../../lib/diagnostics/autoscroll";
  import { lineMatches, splitMatches } from "../../lib/diagnostics/logfilter";
  import { logs } from "../../lib/stores/logs.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";

  const TABS: { value: LogSource; label: string }[] = [
    { value: "runtime", label: "Game" },
    { value: "runtime_errors", label: "Game errors" },
    { value: "launcher", label: "Launcher" },
  ];

  let source = $state<LogSource>("runtime");
  let view = $state<HTMLElement | null>(null);
  let query = $state("");
  /** Pause stops following new lines; they still arrive and are counted. */
  let paused = $state(false);
  /** False once the user scrolls up; new lines then leave the view where it is. */
  let atBottom = $state(true);
  /** The newest line the user has seen at the bottom; later lines count as new. */
  let seenSeq = $state(-1);

  const lines = $derived(logs.lines[source]);
  const visible = $derived(query.trim() ? lines.filter((l) => lineMatches(l.line, query)) : lines);
  const following = $derived(!paused && atBottom);
  const newCount = $derived(following ? 0 : visible.filter((l) => l.seq > seenSeq).length);

  $effect(() => {
    const s = source;
    untrack(() => logs.ensureLoaded(s));
  });

  // After new lines, a tab change or a new filter, follow the bottom unless paused or scrolled up.
  $effect(() => {
    void visible;
    untrack(() => {
      if (following) toBottom();
    });
  });

  function toBottom() {
    seenSeq = lines.at(-1)?.seq ?? -1;
    void tick().then(() => {
      if (view) view.scrollTop = view.scrollHeight;
    });
  }

  /** The new lines pill: show them and follow again. */
  function resume() {
    paused = false;
    atBottom = true;
    toBottom();
  }

  function togglePause() {
    if (paused) resume();
    else paused = true;
  }

  function onscroll() {
    if (!view) return;
    atBottom = isAtBottom(view);
    if (following) seenSeq = lines.at(-1)?.seq ?? -1;
  }

  function pick(v: string) {
    source = (TABS.find((t) => t.value === v)?.value ?? "runtime") as LogSource;
    resume();
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(visible.map((l) => l.line).join("\n"));
      toasts.info(query.trim() ? "Matching lines copied." : "Log copied.");
    } catch {
      toasts.error("Could not copy the log.", "Select the lines and copy them manually.");
    }
  }
</script>

<Panel class="console">
  <div class="head">
    <StencilHeading level={2}>Live log</StencilHeading>
    <div class="tools">
      <TextField class="filter" label="Filter log lines" hideLabel placeholder="Filter lines" bind:value={query} />
      <Button size="sm" class="pause" aria-pressed={paused} onclick={togglePause}>Pause</Button>
      <Button size="sm" disabled={visible.length === 0} onclick={copy}>Copy</Button>
      <Button size="sm" variant="ghost" disabled={lines.length === 0} onclick={() => logs.clear(source)}>Clear</Button>
    </div>
  </div>
  <Tabs fill bind:value={() => source, pick} tabs={TABS}>
    {#snippet content()}
      <div class="frame">
        <!-- Lines are plain text, never HTML; matches are marked with elements built from text. -->
        <!-- The view takes focus so keyboard users can scroll it. -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div class="view" role="log" aria-label="Log lines" tabindex="0" bind:this={view} {onscroll}>
          {#if lines.length === 0}
            <p class="qm-muted none">No log lines yet.</p>
          {:else if visible.length === 0}
            <p class="qm-muted none">No lines match the filter.</p>
          {:else}
            {#each visible as entry (entry.seq)}
              <div class="line">
                {#each splitMatches(entry.line, query) as part, i (i)}{#if part.match}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}
              </div>
            {/each}
          {/if}
        </div>
        {#if newCount > 0}
          <Button size="sm" variant="primary" class="new-lines" onclick={resume}>
            {newCount} new {newCount === 1 ? "line" : "lines"}
          </Button>
        {/if}
      </div>
    {/snippet}
  </Tabs>
</Panel>

<style>
  /* The log takes whatever height the cards above leave. */
  :global(.console) {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--qm-space-2);
    min-height: 240px;
  }

  .head {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-2);
  }

  .tools {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .tools :global(.filter) {
    width: calc(var(--qm-detail-w) * 0.6);
  }

  .tools :global(.pause[aria-pressed="true"]) {
    border-color: var(--qm-accent);
    color: var(--qm-accent-text);
  }

  .frame {
    position: relative;
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .view {
    flex: 1;
    min-height: 0;
    padding: var(--qm-space-2);
    overflow: auto;
    scrollbar-gutter: stable;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  .line {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  mark {
    border-radius: 2px;
    background: var(--qm-accent);
    color: var(--qm-accent-contrast);
  }

  .frame :global(.new-lines) {
    position: absolute;
    bottom: var(--qm-space-3);
    left: 50%;
    z-index: var(--qm-z-panel);
    transform: translateX(-50%);
  }

  .none {
    margin: 0;
  }
</style>
