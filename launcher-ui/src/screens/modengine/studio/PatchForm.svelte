<script lang="ts">
  import CodeEditor from "../../../components/CodeEditor.svelte";
  import { Button, Checkbox, ConfirmDialog, Select, TextField } from "../../../components/ui";
  import type { PatchDraft, StudioPatchKind } from "../../../lib/bindings";
  import { payloadContext } from "../../../lib/engine/studio";
  import { studio } from "../../../lib/stores/studio.svelte";
  import IdPicker from "./IdPicker.svelte";

  const d = $derived(studio.draft as PatchDraft);
  const kind = $derived(d.kind ?? "pattern");
  const anchored = $derived(kind === "pattern" || kind === "regex");
  const context = $derived(
    anchored && studio.file && d.target[0] === studio.file.path ? payloadContext(studio.file.text, studio.selection, d.position) : { above: [], below: [] },
  );
  let removing = $state(false);
  let leaving = $state(false);
  /** A new draft gives the payload editor a new document (and its own undo history). */
  const payloadKey = $derived(`payload-${studio.draftSeq}`);

  const KINDS: { value: StudioPatchKind; label: string }[] = [
    { value: "pattern", label: "Pattern: find lines and add code" },
    { value: "regex", label: "Regex: find with an expression" },
    { value: "module", label: "Module: load a Lua file of the mod" },
    { value: "copy", label: "Copy: add a file's text to a game file" },
  ];
  const POSITIONS = [
    { value: "after", label: "After the lines" },
    { value: "before", label: "Before the lines" },
    { value: "at", label: "In place of the lines" },
  ];
  const COPY_POSITIONS = [
    { value: "append", label: "At the end of the file" },
    { value: "prepend", label: "At the start of the file" },
  ];
  const FAILURE = [
    { value: "", label: "Log it and go on (default)" },
    { value: "skip_patch", label: "Skip this patch" },
    { value: "skip_mod", label: "Skip the mod's other patches in this file" },
    { value: "abort", label: "Stop the game with an error" },
  ];

  function close() {
    if (studio.dirty) leaving = true;
    else studio.closeDraft();
  }
</script>

<form class="form" aria-label="Patch" onsubmit={(e) => (e.preventDefault(), void studio.save())}>
  <div class="grid">
    <Select label="Kind" items={KINDS} bind:value={() => kind, (v) => v && studio.update({ kind: v })} />
    <TextField label="Name" placeholder="What the patch does" bind:value={() => d.name, (v) => studio.update({ name: v })} />
    <TextField label="Id" bind:value={() => d.id, (v) => studio.update({ id: v })} />
    <TextField label="Group" placeholder="Optional" bind:value={() => d.group, (v) => studio.update({ group: v })} />
  </div>

  {#if anchored}
    <p class="target">
      <span class="qm-muted">Game file</span>
      <span class="mono">{d.target[0] ?? "Pick a game file on the left"}</span>
    </p>
    <label class="area">
      <span>{kind === "regex" ? "Expression" : "Lines to find"}</span>
      <textarea
        class="mono"
        rows={Math.min(6, Math.max(2, d.pattern.split("\n").length))}
        placeholder={kind === "regex" ? "A Lua pattern or regex the loader matches" : "Pick lines in the game file, or type them"}
        value={d.pattern}
        oninput={(e) => studio.update({ pattern: e.currentTarget.value })}
      ></textarea>
    </label>
    <div class="grid">
      <Select label="Where the code goes" items={POSITIONS} bind:value={() => d.position || "after", (v) => v && studio.update({ position: v })} />
      <TextField
        label="Times to apply"
        type="number"
        placeholder="Every match"
        value={d.times === null ? "" : String(d.times)}
        oninput={(e: Event) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          studio.update({ times: v === "" ? null : Math.max(1, Math.floor(Number(v))) });
        }}
      />
      <Select label="If it finds nothing" items={FAILURE} bind:value={() => d.on_failure, (v) => studio.update({ on_failure: v ?? "" })} />
      {#if kind === "pattern"}
        <Checkbox label="Match the game's indentation" bind:checked={() => d.match_indent ?? true, (v) => studio.update({ match_indent: v })} />
      {/if}
    </div>

    <div class="payload">
      <span class="label">Code to add</span>
      {#if context.above.length}<pre class="context" aria-label="Game lines above">{context.above.join("\n")}</pre>{/if}
      <div class="payload-editor">
        <CodeEditor
          text={d.payload}
          docKey={payloadKey}
          language="lua"
          label="Code to add"
          onchange={(t) => studio.update({ payload: t })}
          onsave={() => void studio.save()}
        />
      </div>
      {#if context.below.length}<pre class="context" aria-label="Game lines below">{context.below.join("\n")}</pre>{/if}
    </div>

    <IdPicker
      label="Requires"
      help="Patches that must apply first; when one did not, this one is skipped."
      value={d.requires}
      onchange={(ids) => studio.update({ requires: ids })}
    />
  {:else if kind === "module"}
    <TextField label="Lua file of the mod" placeholder="main.lua" value={d.source[0] ?? ""} oninput={(e: Event) => studio.update({ source: [(e.currentTarget as HTMLInputElement).value] })} />
    <TextField label="Load before (game file)" placeholder="main.lua" bind:value={() => d.load_before, (v) => studio.update({ load_before: v })} />
  {:else}
    <TextField label="Game file" placeholder="conf.lua" value={d.target[0] ?? ""} oninput={(e: Event) => studio.update({ target: [(e.currentTarget as HTMLInputElement).value] })} />
    <Select label="Where it goes" items={COPY_POSITIONS} bind:value={() => d.position || "append", (v) => v && studio.update({ position: v })} />
    <TextField
      label="Files of the mod to copy"
      placeholder="extra.lua, more.lua"
      value={d.source.join(", ")}
      oninput={(e: Event) =>
        studio.update({
          source: (e.currentTarget as HTMLInputElement).value
            .split(",")
            .map((s) => s.trim())
            .filter(Boolean),
        })}
    />
  {/if}

  {#if kind !== "module"}
    <IdPicker label="Run before" help="Patches this one goes ahead of in the same file." value={d.before} onchange={(ids) => studio.update({ before: ids })} />
    <IdPicker label="Run after" help="Patches this one follows in the same file." value={d.after} onchange={(ids) => studio.update({ after: ids })} />
  {/if}

  <div class="actions">
    {#if studio.editing}
      <Button variant="ghost" onclick={() => (removing = true)}>Remove</Button>
    {/if}
    <span class="spacer"></span>
    <Button variant="ghost" onclick={close}>Close</Button>
    <Button variant="primary" type="submit" disabled={studio.saving || !d.id.trim()} aria-busy={studio.saving}>
      {studio.editing ? "Save" : "Add to qm.toml"}
    </Button>
  </div>
</form>

<ConfirmDialog
  bind:open={removing}
  title="Remove this patch?"
  body={`${d.name || d.id} is taken out of qm.toml. The rest of the file stays as it is.`}
  confirmLabel="Remove"
  onconfirm={() => studio.editing && studio.remove(studio.editing)}
/>
<ConfirmDialog
  bind:open={leaving}
  title="Close without saving?"
  body="The changes to this patch are lost when it closes."
  confirmLabel="Close without saving"
  onconfirm={() => studio.closeDraft()}
/>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
    min-height: 0;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--qm-space-2) var(--qm-space-3);
    align-items: end;
  }

  .target {
    display: flex;
    gap: var(--qm-space-2);
    margin: 0;
    font-size: var(--qm-text-13);
  }

  .mono {
    font-family: var(--qm-font-mono);
  }

  .area {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    font-size: var(--qm-text-13);
  }

  textarea {
    padding: var(--qm-space-2);
    border: 1px solid var(--qm-border-strong);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
    color: var(--qm-text);
    font-size: var(--qm-text-13);
    resize: vertical;
  }

  .payload {
    display: flex;
    flex-direction: column;
    gap: 0;
  }

  .label {
    margin-bottom: var(--qm-space-1);
    font-size: var(--qm-text-13);
  }

  .payload-editor {
    height: calc(var(--qm-row-h) * 5);
  }

  .context {
    margin: 0;
    padding: 2px var(--qm-space-2);
    overflow-x: auto;
    border: 1px dashed var(--qm-border);
    background: var(--qm-panel);
    color: var(--qm-text-muted);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    line-height: 1.55;
    opacity: 0.8;
  }

  .actions {
    display: flex;
    gap: var(--qm-space-2);
    align-items: center;
  }

  .spacer {
    flex: 1;
  }
</style>
