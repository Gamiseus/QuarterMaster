<script lang="ts">
  import { untrack } from "svelte";
  import CodeEditor, { type EditorProblem } from "../../components/CodeEditor.svelte";
  import { IconClose, IconDanger, IconInfo, IconWarn } from "../../components/icons";
  import { Button, Callout, ConfirmDialog, EmptyState, Kbd } from "../../components/ui";
  import type { EngineDiagnostic } from "../../lib/bindings";
  import { completionFor } from "../../lib/engine/completion";
  import { languageOf, offsetOf, rangeOf } from "../../lib/engine/text";
  import { engine, isDirty, type OpenFile } from "../../lib/stores/engine.svelte";

  let jump = $state<{ offset: number; seq: number } | null>(null);
  let jumpSeq = 0;
  let closing = $state<string | null>(null);

  // A line asked for from elsewhere (a test run's log): shown once, then forgotten.
  $effect(() => {
    const r = engine.jumpRequest;
    if (!r) return;
    untrack(() => {
      const f = engine.files.find((x) => x.path === r.path);
      engine.jumpRequest = null;
      if (!f) return;
      engine.active = r.path;
      jump = { offset: offsetOf(f.text, r.line, null), seq: ++jumpSeq };
    });
  });

  const file = $derived(engine.activeFile);
  const library = $derived(engine.target?.kind === "library" ? engine.target : null);
  const complete = $derived(file && !file.readOnly ? completionFor(file.path, () => engine.symbols) : undefined);

  /** Fast checks of what is typed, then the last full check's findings not already among them. */
  const problems = $derived.by(() => {
    const out: EngineDiagnostic[] = [];
    const seen = new Set<string>();
    const add = (d: EngineDiagnostic) => {
      const key = `${d.path}|${d.line}|${d.message}`;
      if (seen.has(key)) return;
      seen.add(key);
      out.push(d);
    };
    for (const f of engine.files) for (const d of f.problems) add(d);
    for (const d of engine.lint?.diagnostics ?? []) add(d);
    const rank = { error: 0, warning: 1, info: 2 } as const;
    return out.sort((a, b) => rank[a.severity] - rank[b.severity]);
  });

  /** The underlines: the fast checks, and the full check's findings while the file is as it was saved. */
  const marks = $derived.by((): EditorProblem[] => {
    if (!file) return [];
    const list = [...file.problems];
    if (!isDirty(file)) {
      for (const d of engine.lint?.diagnostics ?? []) {
        if (d.path === file.path && !list.some((x) => x.line === d.line && x.message === d.message)) list.push(d);
      }
    }
    return list.flatMap((d) => {
      const r = rangeOf(file.text, d);
      return r ? [{ ...r, severity: d.severity, message: d.message }] : [];
    });
  });

  const counts = $derived({
    errors: problems.filter((p) => p.severity === "error").length,
    warnings: problems.filter((p) => p.severity === "warning").length,
  });

  async function goToProblem(d: EngineDiagnostic) {
    if (!d.path) return;
    await engine.open(d.path).catch(() => {});
    const f = engine.files.find((x) => x.path === d.path);
    if (!f || d.line === null) return;
    jump = { offset: offsetOf(f.text, d.line, d.column), seq: ++jumpSeq };
  }

  function requestClose(f: OpenFile) {
    if (isDirty(f)) closing = f.path;
    else engine.close(f.path);
  }

  function place(d: EngineDiagnostic): string {
    if (!d.path) return "the mod";
    return d.line === null ? d.path : `${d.path}:${d.line}`;
  }
</script>

<section class="pane" aria-label="Editor">
  {#if engine.files.length > 0}
    <div class="tabs" role="tablist" aria-label="Open files">
      {#each engine.files as f (f.path)}
        {@const dirty = isDirty(f)}
        <div class={["tab", { active: f.path === engine.active }]}>
          <button
            type="button"
            role="tab"
            class="tab-name"
            aria-selected={f.path === engine.active}
            title={f.path}
            onclick={() => (engine.active = f.path)}
          >
            {f.path.split("/").pop()}{#if dirty}<span class="unsaved" aria-label="Unsaved changes">*</span>{/if}
          </button>
          <button type="button" class="tab-close" aria-label={`Close ${f.path}`} onclick={() => requestClose(f)}>
            <IconClose />
          </button>
        </div>
      {/each}
      <span class="spacer"></span>
      {#if !library}
        <div class="save">
          <Button size="sm" variant="ghost" disabled={!file || !isDirty(file)} onclick={() => file && engine.saveAndCheck(file.path)}>
            Save <Kbd>Ctrl+S</Kbd>
          </Button>
          <Button size="sm" variant="ghost" disabled={!engine.hasUnsaved} onclick={() => engine.saveAll()}>Save all</Button>
        </div>
      {/if}
    </div>
  {/if}

  {#if library}
    <Callout tone="info" title="Read-only">
      Library mods are not edited in place. Copy {library.name} to the workspace to change it.
      {#snippet action()}
        <Button size="sm" onclick={() => engine.copyToWorkspace(library).catch(() => {})}>Copy to workspace</Button>
      {/snippet}
    </Callout>
  {/if}

  {#if file?.changedOnDisk}
    <Callout tone="warn" title="Changed on disk">
      {file.path} changed outside the Mod Engine while you were editing it.
      {#snippet action()}
        <div class="row">
          <Button size="sm" onclick={() => engine.reload(file.path)}>Reload</Button>
          <Button size="sm" variant="ghost" onclick={() => engine.save(file.path, true)}>Keep mine</Button>
        </div>
      {/snippet}
    </Callout>
  {/if}

  <div class="editor">
    {#if file}
      <CodeEditor
        text={file.text}
        docKey={`${file.path}#${file.docVersion}`}
        language={languageOf(file.path)}
        label={`${file.readOnly ? "Viewing" : "Editing"} ${file.path}`}
        readOnly={file.readOnly}
        problems={marks}
        {complete}
        {jump}
        onchange={(t) => engine.edit(file.path, t)}
        onsave={() => engine.saveAndCheck(file.path)}
      />
    {:else}
      <EmptyState title="No file open" body="Pick a file on the left to open it here." />
    {/if}
  </div>

  {#if !library}
    <section class="problems" aria-label="Problems">
      <header class="problems-head">
        <h2 class="qm-stencil head">Problems</h2>
        <span class="qm-muted count">
          {#if problems.length === 0}None{:else}{counts.errors} {counts.errors === 1 ? "error" : "errors"}, {counts.warnings}
            {counts.warnings === 1 ? "warning" : "warnings"}{/if}
        </span>
        {#if engine.lint}
          <span class="qm-muted summary">
            {#if engine.lint.game_checked}
              Last full check: {engine.lint.patch_count} {engine.lint.patch_count === 1 ? "patch" : "patches"},
              {engine.lint.matched_patches} found their place in the game{#if engine.lint.zero_match_patches > 0}, {engine.lint.zero_match_patches}
                found none{/if}.
            {:else}
              Last full check: patches were not tried against the game; its files are not read yet.
            {/if}
          </span>
        {/if}
      </header>
      {#if file && languageOf(file.path) === "lua" && !file.luaChecked}
        <p class="note qm-muted">Lua is checked with the game's own Lua, which was not found. Set the game folder in Settings to check it.</p>
      {/if}
      {#if problems.length > 0}
        <ul class="list">
          {#each problems as d, i (i)}
            <li>
              <button type="button" class={["problem", `s-${d.severity}`]} disabled={!d.path} onclick={() => goToProblem(d)}>
                <span class="icon" aria-hidden="true">
                  {#if d.severity === "error"}<IconDanger />{:else if d.severity === "warning"}<IconWarn />{:else}<IconInfo />{/if}
                </span>
                <span class="qm-visually-hidden">{d.severity}:</span>
                <span class="message">{d.message}</span>
                <span class="where qm-muted">{place(d)}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}
</section>

<ConfirmDialog
  bind:open={() => closing !== null, (v) => !v && (closing = null)}
  title="Close without saving?"
  body={closing ? `Your changes to ${closing} are lost when it closes.` : ""}
  confirmLabel="Close without saving"
  onconfirm={() => closing && engine.close(closing)}
/>

<style>
  .pane {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    min-width: 0;
    min-height: 0;
  }

  .tabs {
    display: flex;
    flex: none;
    align-items: stretch;
    gap: 2px;
    overflow-x: auto;
    border-bottom: 1px solid var(--qm-border);
  }

  .tab {
    display: flex;
    align-items: center;
    border: 1px solid transparent;
    border-bottom: 0;
    border-radius: var(--qm-radius-sm) var(--qm-radius-sm) 0 0;
    color: var(--qm-text-muted);
  }

  .tab.active {
    border-color: var(--qm-border);
    background: var(--qm-panel);
    color: var(--qm-text);
    box-shadow: inset 0 2px 0 var(--qm-accent);
  }

  .tab-name {
    padding: var(--qm-space-2) var(--qm-space-1) var(--qm-space-2) var(--qm-space-3);
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    font-size: var(--qm-text-13);
    white-space: nowrap;
    cursor: pointer;
  }

  .unsaved {
    margin-left: 2px;
    color: var(--qm-accent-text);
    font-weight: 700;
  }

  .tab-close {
    display: inline-flex;
    margin-right: var(--qm-space-1);
    padding: 2px;
    border: 0;
    border-radius: var(--qm-radius-sm);
    background: none;
    color: var(--qm-text-muted);
    cursor: pointer;
  }

  .tab-close:hover {
    background: var(--qm-raised);
    color: var(--qm-text);
  }

  .spacer {
    flex: 1;
  }

  .save {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--qm-space-1);
  }

  .row {
    display: flex;
    gap: var(--qm-space-2);
  }

  .editor {
    flex: 1;
    min-height: 0;
  }

  .problems {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: var(--qm-space-1);
    max-height: 30%;
    min-height: calc(var(--qm-row-h) * 1.5);
    border-top: 1px solid var(--qm-border);
    padding-top: var(--qm-space-2);
  }

  .problems-head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--qm-space-1) var(--qm-space-3);
  }

  .head {
    font-size: var(--qm-text-13);
    color: var(--qm-text-muted);
  }

  .count,
  .summary,
  .note {
    font-size: var(--qm-text-12);
  }

  .note {
    margin: 0;
  }

  .list {
    position: relative;
    margin: 0;
    padding: 0;
    overflow: auto;
    list-style: none;
  }

  .problem {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: baseline;
    gap: var(--qm-space-2);
    width: 100%;
    padding: var(--qm-space-1) var(--qm-space-2);
    border: 0;
    border-radius: var(--qm-radius-sm);
    background: none;
    color: var(--qm-text);
    font: inherit;
    font-size: var(--qm-text-13);
    text-align: left;
    cursor: pointer;
  }

  .problem:hover:not(:disabled) {
    background: var(--qm-raised);
  }

  .problem:disabled {
    cursor: default;
  }

  .icon {
    display: inline-flex;
    align-self: center;
  }

  .s-error .icon {
    color: var(--qm-danger);
  }

  .s-warning .icon {
    color: var(--qm-warn);
  }

  .s-info .icon {
    color: var(--qm-info);
  }

  .where {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    white-space: nowrap;
  }

</style>
