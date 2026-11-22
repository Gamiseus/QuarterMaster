<script lang="ts">
  import CodeEditor, { type LineMark } from "../../../components/CodeEditor.svelte";
  import { Badge, Button, Callout, EmptyState, Menu, Select, Switch } from "../../../components/ui";
  import type { StudioPatch, StudioPatchKind } from "../../../lib/bindings";
  import { matchText } from "../../../lib/engine/studio";
  import { engine } from "../../../lib/stores/engine.svelte";
  import { sameRef, studio } from "../../../lib/stores/studio.svelte";
  import PatchForm from "./PatchForm.svelte";

  interface Props {
    folder: string;
  }

  let { folder }: Props = $props();

  let pane = $state<"game" | "result">("game");
  let jump = $state<{ offset: number; seq: number } | null>(null);
  let jumpSeq = 0;

  $effect(() => {
    const f = folder;
    studio.load(f).catch(() => {});
  });

  const fileItems = $derived(
    (studio.files?.files ?? []).map((f) => ({ value: f.path, label: f.changed_by > 0 ? `${f.path} (changed by ${f.changed_by})` : f.path })),
  );

  const gameMarks = $derived.by((): LineMark[] => {
    const marks: LineMark[] = [];
    if (studio.withMods && studio.file?.with_mods) {
      for (const r of studio.file.runs) marks.push({ from: r.start, to: r.end, kind: "mod", title: `${r.patch_name || r.patch_id} (${r.mod_id})` });
    }
    if (studio.selection) marks.push({ from: studio.selection.from, to: studio.selection.to, kind: "chosen" });
    return marks;
  });

  const resultMarks = $derived(
    (studio.preview?.changed ?? []).map((r): LineMark => ({ from: r.start, to: r.end, kind: r.mine ? "mine" : "changed" })),
  );

  const outcome = $derived.by(() => {
    const p = studio.preview;
    if (!p) return null;
    const o = p.outcome;
    switch (o.kind) {
      case "applied":
        return { tone: "ok" as const, text: o.matched === 1 ? "Applies in 1 place" : `Applies in ${o.matched} places` };
      case "no_match":
        return { tone: "danger" as const, text: "Finds nothing in the file: the patch would not apply" };
      case "wrong_count":
        return { tone: "warn" as const, text: `Found ${o.matched} places, times asks for ${o.expected}` };
      case "skipped":
        return { tone: "warn" as const, text: "Skipped" };
      case "refused":
        return { tone: "danger" as const, text: "The engine refused it" };
      default:
        return { tone: "warn" as const, text: o.messages[0] ?? "The patch cannot be tried yet" };
    }
  });

  const KIND_LABEL: Record<StudioPatchKind, string> = { pattern: "Pattern", regex: "Regex", module: "Module", copy: "Copy" };

  function newPatch(kind: StudioPatchKind) {
    if (studio.dirty) return;
    studio.newPatch(kind);
  }

  async function open(p: StudioPatch) {
    if (studio.dirty) return;
    await studio.edit(p);
    pane = "game";
    if (studio.selection) goToLine(studio.selection.from);
  }

  /** Scrolls the game file to the start of `line` (1-based). */
  function goToLine(line: number) {
    const f = studio.file;
    if (!f) return;
    jump = { offset: f.text.split("\n").slice(0, line - 1).join("\n").length + (line > 1 ? 1 : 0), seq: ++jumpSeq };
  }
</script>

<div class="studio">
  <section class="left" aria-label="Game code">
    <div class="bar">
      <div class="picker">
        <Select
          label="Game file"
          hideLabel
          placeholder={studio.files?.game_ready === false ? "The game's files are not read yet" : "Pick a game file"}
          items={fileItems}
          disabled={!studio.files?.game_ready}
          bind:value={() => studio.file?.path, (v) => v && studio.openFile(v).catch(() => {})}
        />
      </div>
      <Switch label="Other mods' changes" checked={studio.withMods} onCheckedChange={(v: boolean) => void studio.setWithMods(v)} />
      <div class="seg" role="group" aria-label="Show">
        <button type="button" aria-pressed={pane === "game"} onclick={() => (pane = "game")}>Game file</button>
        <button type="button" aria-pressed={pane === "result"} disabled={!studio.draft} onclick={() => (pane = "result")}>Result</button>
      </div>
    </div>

    {#if studio.files?.game_ready === false}
      <Callout tone="info" title="No game files yet">QuarterMaster reads the game's code once the game folder is set in Settings; the Patch Studio shows it from there.</Callout>
    {/if}

    {#if pane === "game"}
      {#if studio.file}
        <div class="sel-bar" aria-live="polite">
          {#if studio.selection}
            <span class="mono">Lines {studio.selection.from}{studio.selection.to !== studio.selection.from ? `-${studio.selection.to}` : ""}</span>
            {#if studio.matches}
              {@const m = matchText(studio.matches.count)}
              <Badge tone={m.tone}>{m.text}</Badge>
              {#if studio.matches.count > 1}
                <span class="places">
                  {#each studio.matches.lines as l (l)}<button type="button" class="link" onclick={() => goToLine(l)}>line {l}</button>{/each}
                </span>
              {/if}
            {/if}
            <span class="spacer"></span>
            <span class="tools" role="group" aria-label="Chosen lines">
              <Button size="sm" variant="ghost" onclick={() => studio.extend(true)}>Extend up</Button>
              <Button size="sm" variant="ghost" onclick={() => studio.extend()}>Extend down</Button>
              <Button size="sm" variant="ghost" disabled={studio.selection.from === studio.selection.to} onclick={() => studio.shrink()}>Shrink</Button>
            </span>
          {:else}
            <span class="qm-muted">Click a line, or drag over several, to choose what the patch finds.</span>
          {/if}
        </div>
        {#if studio.withMods && !studio.file.with_mods}
          <p class="qm-muted note">No mod in the active profile changes this file.</p>
        {/if}
        <div class="code">
          <CodeEditor
            text={studio.file.text}
            docKey={`game:${studio.file.path}:${studio.file.with_mods}`}
            language="lua"
            label={`Game file ${studio.file.path}`}
            readOnly
            lineMarks={gameMarks}
            {jump}
            onselectlines={(r) => studio.select(r)}
          />
        </div>
      {:else}
        <EmptyState title="Pick a game file" body="Choose a file of the game's code above, then the lines your patch finds." />
      {/if}
    {:else if studio.preview}
      <div class="sel-bar" aria-live="polite">
        {#if outcome}<Badge tone={outcome.tone}>{outcome.text}</Badge>{/if}
        {#if studio.preview.compile_error}
          <Badge tone="danger">Lua error, {studio.preview.compile_error}</Badge>
        {:else if studio.preview.compile_checked && studio.preview.outcome.kind === "applied"}
          <Badge tone="ok">Compiles</Badge>
        {/if}
        {#if studio.previewing}<span class="qm-muted">Working it out...</span>{/if}
      </div>
      {#if studio.preview.mod_in_base}
        <p class="qm-muted note">The active profile loads this mod, so its saved patches are already in the file; the result shows this patch on top of them.</p>
      {/if}
      {#each studio.preview.outcome.messages as m, i (i)}<p class="note warn">{m}</p>{/each}
      <div class="code">
        <CodeEditor
          text={studio.preview.text}
          docKey={`result:${studio.preview.target}`}
          language="lua"
          label={`Result ${studio.preview.target}`}
          readOnly
          lineMarks={resultMarks}
        />
      </div>
    {:else}
      <EmptyState title="No result yet" body="Fill in the patch on the right; the result shows the file as the game would load it." />
    {/if}
  </section>

  <section class="right" aria-label="Patch Studio">
    {#if studio.draft}
      <h2 class="qm-stencil head">{studio.editing ? "Edit patch" : "New patch"}</h2>
      <PatchForm />
    {:else}
      <div class="list-head">
        <h2 class="qm-stencil head">Patches of {engine.projects?.projects.find((p) => p.folder === folder)?.name ?? folder}</h2>
        <Menu
          align="end"
          items={[
            { label: "Pattern patch", description: "Find lines and add code", onselect: () => newPatch("pattern") },
            { label: "Regex patch", description: "Find with an expression", onselect: () => newPatch("regex") },
            { label: "Module patch", description: "Load a Lua file of the mod", onselect: () => newPatch("module") },
            { label: "Copy patch", description: "Add a file's text to a game file", onselect: () => newPatch("copy") },
          ]}
        >
          {#snippet trigger({ props })}
            <Button {...props} size="sm" variant="primary">New patch</Button>
          {/snippet}
        </Menu>
      </div>
      {#if (studio.patches?.patches.length ?? 0) === 0}
        <p class="qm-muted">This mod has no patches yet. Pick lines in a game file, then New patch.</p>
      {:else}
        <ul class="patches">
          {#each studio.patches?.patches ?? [] as p (`${p.at.file}:${p.at.form}:${p.at.kind}:${p.at.index}`)}
            {@const current = studio.editing !== null && sameRef(studio.editing, p.at)}
            <li class={["patch", { current }]}>
              <div class="meta">
                <span class="name">{p.draft.name || p.draft.id}</span>
                <span class="qm-muted small">{KIND_LABEL[p.at.kind]}{#if p.draft.target[0]}, {p.draft.target[0]}{/if}{#if p.at.file !== "qm.toml"}, in {p.at.file}{/if}</span>
              </div>
              {#if p.editable}
                <Button size="sm" variant="ghost" aria-label={`Edit ${p.draft.name || p.draft.id}`} onclick={() => open(p)}>Edit</Button>
              {:else}
                <span class="qm-muted small" title="The Patch Studio writes qm.toml; edit this file in the Files view.">Read-only</span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </section>
</div>

<style>
  .studio {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(20rem, 28rem);
    gap: var(--qm-space-4);
    flex: 1;
    min-height: 0;
  }

  .left,
  .right {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    min-width: 0;
    min-height: 0;
  }

  .right {
    /* Contains the hidden labels too, so nothing in it reaches past the screen. */
    position: relative;
    overflow: auto;
    padding-right: var(--qm-space-1);
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2) var(--qm-space-4);
  }

  .picker {
    flex: 1;
    min-width: 14rem;
    max-width: 28rem;
  }

  .seg {
    display: inline-flex;
    border: 1px solid var(--qm-border-strong);
    border-radius: var(--qm-radius-sm);
    overflow: hidden;
  }

  .seg button {
    padding: var(--qm-space-1) var(--qm-space-3);
    border: 0;
    background: none;
    color: var(--qm-text-muted);
    font: inherit;
    font-size: var(--qm-text-13);
    cursor: pointer;
  }

  .seg button[aria-pressed="true"] {
    background: color-mix(in srgb, var(--qm-accent) 18%, transparent);
    color: var(--qm-text);
  }

  .seg button:disabled {
    cursor: default;
    opacity: 0.5;
  }

  .sel-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
    min-height: var(--qm-row-h);
    font-size: var(--qm-text-13);
  }

  .tools {
    display: inline-flex;
    flex: none;
    gap: 2px;
  }

  .places {
    display: inline-flex;
    gap: var(--qm-space-2);
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

  .mono {
    font-family: var(--qm-font-mono);
  }

  .spacer {
    flex: 1;
  }

  .note {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .warn {
    color: var(--qm-warn);
  }

  .code {
    flex: 1;
    min-height: 0;
  }

  .head {
    margin: 0;
    font-size: var(--qm-text-13);
    color: var(--qm-text-muted);
  }

  .list-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-2);
  }

  .patches {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .patch {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
    padding: var(--qm-space-2);
    border: 1px solid transparent;
    border-radius: var(--qm-radius-sm);
  }

  .patch.current {
    border-color: var(--qm-accent);
  }

  .meta {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .name {
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .small {
    font-size: var(--qm-text-12);
  }
</style>
