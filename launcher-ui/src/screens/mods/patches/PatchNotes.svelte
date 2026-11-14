<script lang="ts">
  import { Button } from "../../../components/ui";
  import FileViewDialog from "../../../components/FileViewDialog.svelte";
  import StatusIcon from "../../../components/StatusIcon.svelte";
  import { call } from "../../../lib/api";
  import { commands } from "../../../lib/bindings";
  import type { DetectedConflict } from "../../../lib/bindings";
  import { explainConflict } from "../../../lib/patches/explain";
  import { resolver } from "../../../lib/stores/resolver.svelte";
  import { openMod } from "../mods-view.svelte";

  interface Props {
    /** Conflicts no file view lists (a module, a `[vars]` name). */
    unlisted: DetectedConflict[];
    /** Order rules and places the simulation does not use. */
    orderList: string[];
    /** Adjusted copies the game skips while their mod is off. */
    waiting: string[];
    /** Why patched files were not (all) compiled with the game's Lua. */
    compileNote?: string | null;
    modNames: Record<string, string>;
    /** Ids of the conflicts dismissed in this profile: listed apart and not counted. */
    dismissed?: string[];
    canMutate?: boolean;
  }

  let { unlisted, orderList, waiting, compileNote = null, modNames, dismissed = [], canMutate = true }: Props = $props();

  const hidden = $derived(new Set(dismissed));
  const openList = $derived(unlisted.filter((c) => !hidden.has(c.id)));
  const dismissedList = $derived(unlisted.filter((c) => hidden.has(c.id)));
  let showDismissed = $state(false);
  let busy = $state(false);

  /** Dismisses a conflict in this profile, or lists it as open again. */
  async function dismiss(c: DetectedConflict, on: boolean) {
    if (busy) return;
    busy = true;
    try {
      await call(commands.conflictDismiss(c.id, on));
    } catch {
      // call() showed the error.
    } finally {
      busy = false;
    }
  }

  /** A conflict about a game file names it: the file can be read even though no patch changes it. */
  const isFile = (c: DetectedConflict) => /\.[a-z0-9]+$/i.test(c.target_file);
  let viewOpen = $state(false);
  let viewFile = $state("");
  function view(c: DetectedConflict) {
    viewFile = c.target_file;
    viewOpen = true;
  }

  const OPEN_KEY = "qm.patches.notes.open";
  function stored(): boolean {
    try {
      return localStorage.getItem(OPEN_KEY) === "1";
    } catch {
      return false;
    }
  }
  /** One line until opened, so the notes never push the file out of view. */
  let open = $state(stored());
  function toggle() {
    open = !open;
    try {
      localStorage.setItem(OPEN_KEY, open ? "1" : "0");
    } catch {
      // The choice then lasts for this visit only.
    }
  }

  const modName = (id: string) => modNames[id] ?? id;
  /** The installed mods a conflict names, to open from its row. */
  const modsOf = (c: DetectedConflict) => [...new Set([c.mod_a_id, c.mod_b_id].filter((m): m is string => !!m && m in modNames && m !== "qm_glue"))];

  const count = (n: number, one: string, many: string) => (n === 1 ? `1 ${one}` : `${n} ${many}`);
  const parts = $derived(
    [
      openList.length > 0 ? count(openList.length, "conflict outside the files", "conflicts outside the files") : "",
      dismissedList.length > 0 ? count(dismissedList.length, "dismissed conflict", "dismissed conflicts") : "",
      orderList.length > 0 ? count(orderList.length, "order rule or place not used", "order rules or places not used") : "",
      waiting.length > 0 ? count(waiting.length, "adjusted copy waiting", "adjusted copies waiting") : "",
      compileNote ? "a note on the compile check" : "",
    ].filter((p) => p !== ""),
  );
  const total = $derived(unlisted.length + orderList.length + waiting.length + (compileNote ? 1 : 0));
</script>

{#if total > 0}
  <section class={["notes", { open }]} aria-label="Notes on this profile's patches" data-testid="patch-notes">
    <div class="bar">
      <StatusIcon tone={openList.length > 0 ? "warn" : "info"} />
      <span class="summary">{parts.join(", ")}</span>
      <Button size="sm" variant="ghost" aria-expanded={open} onclick={toggle}>{open ? "Hide" : "Show"}</Button>
    </div>
    {#if open}
      <div class="body">
        {#if openList.length > 0}
          <div class="group" aria-label="Conflicts not in a file" data-testid="unlisted-conflicts">
            <p class="title">{openList.length === 1 ? "1 conflict is" : `${openList.length} conflicts are`} not tied to a line in a file:</p>
            <p class="qm-muted hint">
              They come from whole modules or shared names, so no file view shows them. Each one names the mods involved.
              Dismiss one you have looked at: it stays listed below and stops counting in this profile.
            </p>
            <ul>
              {#each openList as c (c.id)}
                <li>
                  <span class="text">{explainConflict(c, { records: [], modName })}</span>
                  <span class="acts">
                    {#if (c.suggestions?.length ?? 0) > 0}
                      <Button size="sm" onclick={() => resolver.show(c.id)}>Resolve...</Button>
                    {/if}
                    {#if isFile(c)}
                      <Button size="sm" variant="ghost" onclick={() => view(c)}>View {c.target_file}</Button>
                    {/if}
                    {#each modsOf(c) as m (m)}
                      <Button size="sm" variant="ghost" onclick={() => openMod(m, "patches")}>Open {modName(m)}</Button>
                    {/each}
                    {#if c.severity !== "error"}
                      <Button size="sm" variant="ghost" disabled={!canMutate || busy} onclick={() => dismiss(c, true)}>Dismiss</Button>
                    {/if}
                  </span>
                </li>
              {/each}
            </ul>
          </div>
        {/if}
        {#if dismissedList.length > 0}
          <div class="group" aria-label="Dismissed conflicts" data-testid="dismissed-conflicts">
            <p class="title">
              {dismissedList.length === 1 ? "1 dismissed conflict" : `${dismissedList.length} dismissed conflicts`}
              <Button size="sm" variant="ghost" aria-expanded={showDismissed} onclick={() => (showDismissed = !showDismissed)}>
                {showDismissed ? "Hide" : "Show"}
              </Button>
            </p>
            {#if showDismissed}
              <ul>
                {#each dismissedList as c (c.id)}
                  <li>
                    <span class="text qm-muted">{explainConflict(c, { records: [], modName })}</span>
                    <span class="acts">
                      {#if isFile(c)}
                        <Button size="sm" variant="ghost" onclick={() => view(c)}>View {c.target_file}</Button>
                      {/if}
                      <Button size="sm" variant="ghost" disabled={!canMutate || busy} onclick={() => dismiss(c, false)}>Show again</Button>
                    </span>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        {/if}
        {#if orderList.length > 0}
          <div class="group" aria-label="Order notes" data-testid="order-notes">
            <p class="title">{orderList.length === 1 ? "1 order rule or place is" : `${orderList.length} order rules or places are`} not used:</p>
            <ul>
              {#each orderList as note, i (`${note}:${i}`)}<li><span class="text">{note}</span></li>{/each}
            </ul>
          </div>
        {/if}
        {#each waiting as note, i (`${note}:${i}`)}
          <p class="line" role="note" data-testid="waiting-note">{note}</p>
        {/each}
        {#if compileNote}
          <p class="line" role="note" data-testid="compile-note">{compileNote}</p>
        {/if}
      </div>
    {/if}
  </section>
{/if}

<FileViewDialog bind:open={viewOpen} title={viewFile} files={[{ source: "vanilla", path: viewFile, label: "The game's file" }]} />

<style>
  .notes {
    flex: none;
    margin-top: var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-md);
    background: var(--qm-panel);
    font-size: var(--qm-text-13);
  }

  .bar {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
    padding: var(--qm-space-1) var(--qm-space-2) var(--qm-space-1) var(--qm-space-3);
  }

  .summary {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Opened, the notes scroll inside a quarter of the window: the file below keeps its room. */
  .body {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
    max-height: 26vh;
    padding: var(--qm-space-2) var(--qm-space-3) var(--qm-space-3);
    border-top: 1px solid var(--qm-border);
    overflow-y: auto;
  }

  p {
    margin: 0;
  }

  .title {
    font-weight: 600;
  }

  .hint {
    font-size: var(--qm-text-12);
  }

  ul {
    display: flex;
    flex-direction: column;
    margin: var(--qm-space-1) 0 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: var(--qm-space-3);
    padding: var(--qm-space-1) 0;
    border-bottom: 1px solid var(--qm-border);
  }

  li:last-child {
    border-bottom: 0;
  }

  .text {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .acts {
    display: flex;
    flex: 0 1 auto;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--qm-space-1);
  }

  .line {
    color: var(--qm-text-muted);
  }
</style>
