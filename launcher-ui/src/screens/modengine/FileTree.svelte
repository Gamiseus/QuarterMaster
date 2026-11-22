<script lang="ts">
  import { untrack } from "svelte";
  import { ConfirmDialog, IconButton } from "../../components/ui";
  import { IconChevronDown, IconChevronRight, IconEdit, IconFolderPlus, IconPlus, IconTrash } from "../../components/icons";
  import { cleanName, parentOf, treeRows } from "../../lib/engine/tree";
  import { engine, isDirty } from "../../lib/stores/engine.svelte";
  import NameDialog from "../profiles/NameDialog.svelte";

  /** A mod with more entries than this opens with its folders closed. */
  const OPEN_LIMIT = 40;

  let collapsed = $state(new Set<string>());
  /** The row the folder actions apply to; a file's row means its folder for New. */
  let picked = $state<string | null>(null);
  let creating = $state<"file" | "folder" | null>(null);
  let renaming = $state(false);
  let deleting = $state(false);

  const editable = $derived(engine.target?.kind === "workspace");
  const rows = $derived(treeRows(engine.tree, collapsed));
  const pickedEntry = $derived(engine.tree.find((e) => e.path === picked) ?? null);
  const baseFolder = $derived(pickedEntry ? (pickedEntry.folder ? pickedEntry.path : parentOf(pickedEntry.path)) : "");
  const dirtyPaths = $derived(new Set(engine.files.filter(isDirty).map((f) => f.path)));

  // Each mod starts with its folders open, or closed when it has many files.
  let shownFor: unknown = null;
  $effect(() => {
    const t = engine.target;
    const tree = engine.tree;
    if (t === shownFor || tree.length === 0) return;
    shownFor = t;
    picked = null;
    collapsed = new Set(tree.length > OPEN_LIMIT ? tree.filter((e) => e.folder).map((e) => e.path) : []);
  });

  // The open file's folders open with it, so it can be found in the tree.
  $effect(() => {
    const path = engine.active;
    if (!path) return;
    untrack(() => {
      const parts = path.split("/");
      const next = new Set(collapsed);
      let changed = false;
      for (let i = 1; i < parts.length; i++) changed = next.delete(parts.slice(0, i).join("/")) || changed;
      if (changed) collapsed = next;
    });
  });

  function toggle(path: string) {
    const next = new Set(collapsed);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    collapsed = next;
  }

  function choose(path: string, folder: boolean) {
    picked = path;
    if (folder) toggle(path);
    // An image of a workspace mod opens in the sprite line-up.
    else if (editable && /\.(png|jpe?g|bmp)$/i.test(path)) engine.openSprite(path);
    else engine.open(path).catch(() => {});
  }

  async function create(name: string) {
    const clean = cleanName(name);
    if (!clean || !creating) return;
    const path = baseFolder ? `${baseFolder}/${clean}` : clean;
    await engine.create(path, creating === "folder");
    picked = path;
  }

  async function rename(name: string) {
    const clean = cleanName(name);
    if (!clean || !pickedEntry) return;
    const parent = parentOf(pickedEntry.path);
    const to = parent ? `${parent}/${clean}` : clean;
    if (to === pickedEntry.path) return;
    await engine.rename(pickedEntry.path, to);
    picked = to;
  }

  async function remove() {
    if (!pickedEntry) return;
    await engine.remove(pickedEntry.path);
    picked = null;
  }
</script>

<section class="files" aria-label="Files">
  <header class="bar">
    <h2 class="qm-stencil head">Files</h2>
    {#if editable}
      <div class="tools">
        <IconButton icon={IconPlus} label="New file" onclick={() => (creating = "file")} />
        <IconButton icon={IconFolderPlus} label="New folder" onclick={() => (creating = "folder")} />
        <IconButton icon={IconEdit} label="Rename" disabled={!pickedEntry} onclick={() => (renaming = true)} />
        <IconButton icon={IconTrash} label="Move to the Recycle Bin" disabled={!pickedEntry} onclick={() => (deleting = true)} />
      </div>
    {/if}
  </header>
  {#if rows.length === 0}
    <p class="qm-muted empty">{editable ? "This mod has no files yet. Use New file to start." : "No files."}</p>
  {:else}
    <ul class="tree" role="tree" aria-label="Files of the mod">
      {#each rows as r (r.entry.path)}
        {@const open = !collapsed.has(r.entry.path)}
        <li role="none">
          <button
            type="button"
            role="treeitem"
            aria-selected={picked === r.entry.path}
            aria-expanded={r.entry.folder ? open : undefined}
            class={["row", { picked: picked === r.entry.path, active: engine.active === r.entry.path, folder: r.entry.folder }]}
            style:padding-left={`calc(var(--qm-space-2) + ${r.depth} * var(--qm-space-4))`}
            onclick={() => choose(r.entry.path, r.entry.folder)}
          >
            {#if r.entry.folder}
              <span class="chev" aria-hidden="true">{#if open}<IconChevronDown />{:else}<IconChevronRight />{/if}</span>
            {:else}
              <span class={["dot", `k-${r.entry.kind}`]} aria-hidden="true"></span>
            {/if}
            <span class="label">{r.name}</span>
            {#if dirtyPaths.has(r.entry.path)}<span class="unsaved" aria-label="Unsaved changes">*</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
    {#if engine.treeTruncated}<p class="qm-muted empty">Some files are not listed: the mod has too many.</p>{/if}
  {/if}
</section>

<NameDialog
  bind:open={() => creating !== null, (v) => !v && (creating = null)}
  title={creating === "folder" ? "New folder" : "New file"}
  label={creating === "folder" ? "Folder name" : "File name"}
  description={baseFolder ? `In ${baseFolder}/. A name with / makes folders too.` : "In the mod's folder. A name with / makes folders too."}
  submitLabel="Create"
  onsubmit={create}
/>
<NameDialog
  bind:open={renaming}
  title="Rename"
  label="New name"
  initial={pickedEntry ? pickedEntry.path.slice(parentOf(pickedEntry.path).length ? parentOf(pickedEntry.path).length + 1 : 0) : ""}
  submitLabel="Rename"
  onsubmit={rename}
/>
<ConfirmDialog
  bind:open={deleting}
  title="Move to the Recycle Bin?"
  body={pickedEntry ? `${pickedEntry.path}${pickedEntry.folder ? " and everything in it" : ""} goes to the Recycle Bin. You can restore it from there.` : ""}
  confirmLabel="Move to the Recycle Bin"
  onconfirm={remove}
/>

<style>
  .files {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--qm-space-2);
    min-height: 0;
  }

  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-2);
  }

  .head {
    font-size: var(--qm-text-13);
    color: var(--qm-text-muted);
  }

  .tools {
    display: flex;
    gap: 2px;
  }

  .tree {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 0;
    overflow: auto;
    list-style: none;
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
    width: 100%;
    min-height: calc(var(--qm-row-h) * 0.8);
    padding-right: var(--qm-space-2);
    border: 1px solid transparent;
    border-radius: var(--qm-radius-sm);
    background: none;
    color: var(--qm-text);
    font: inherit;
    font-size: var(--qm-text-13);
    text-align: left;
    cursor: pointer;
  }

  .row:hover {
    background: var(--qm-raised);
  }

  .row.picked {
    border-color: var(--qm-border-strong);
  }

  .row.active {
    background: color-mix(in srgb, var(--qm-accent) 12%, transparent);
  }

  .row.folder {
    color: var(--qm-text-muted);
  }

  .chev {
    display: inline-flex;
    width: var(--qm-icon-size);
  }

  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    margin: 0 4px;
    border-radius: 50%;
    background: var(--qm-border-strong);
  }

  .k-lua {
    background: var(--qm-info);
  }

  .k-toml {
    background: var(--qm-accent);
  }

  .k-json {
    background: var(--qm-warn);
  }

  .k-markdown,
  .k-text {
    background: var(--qm-text-muted);
  }

  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .unsaved {
    margin-left: auto;
    color: var(--qm-accent-text);
    font-weight: 700;
  }

  .empty {
    margin: 0;
    font-size: var(--qm-text-12);
  }
</style>
