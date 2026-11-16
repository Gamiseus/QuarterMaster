<script lang="ts">
  import FileViewDialog from "../../../components/FileViewDialog.svelte";
  import { IconChevronRight } from "../../../components/icons";
  import { Button, Section, Skeleton, TextField } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type ModFileList } from "../../../lib/bindings";
  import { fileRows, filterFiles, folderPaths, kindCounts, type FileKind } from "../../../lib/modlist/modfiles";
  import { formatBytes } from "../../../lib/saves/format";

  interface Props {
    modId: string;
    modName: string;
    /** The version in use; a change reads the list again. */
    version: string;
  }

  let { modId, modName, version }: Props = $props();

  /** A mod with more files than this starts with its folders closed. */
  const OPEN_LIMIT = 24;
  /** The filter appears from this many files. */
  const FILTER_FROM = 12;
  /** Most matches a filter shows. */
  const MATCH_LIMIT = 200;

  type State = { status: "loading" } | { status: "ready"; list: ModFileList } | { status: "error"; message: string };
  let data = $state<State>({ status: "loading" });
  let open = $state(new Set<string>());
  let query = $state("");

  $effect(() => {
    const id = modId;
    void version;
    let stale = false;
    data = { status: "loading" };
    query = "";
    call(commands.modFiles(id), { expect: ["not_found", "invalid_input", "io"] }).then(
      (list) => {
        if (stale) return;
        open = new Set(list.files.length <= OPEN_LIMIT ? folderPaths(list.files) : []);
        data = { status: "ready", list };
      },
      (e: unknown) => {
        if (!stale) data = { status: "error", message: e instanceof Error ? e.message : String(e) };
      },
    );
    return () => {
      stale = true;
    };
  });

  const files = $derived(data.status === "ready" ? data.list.files : []);
  const filtering = $derived(query.trim() !== "");
  const matches = $derived(filtering ? filterFiles(files, query) : []);
  const rows = $derived(filtering ? matches.slice(0, MATCH_LIMIT) : fileRows(files, open));
  const folders = $derived(folderPaths(files));

  const KIND_WORDS: Record<FileKind, [string, string]> = {
    code: ["code file", "code files"],
    image: ["image", "images"],
    sound: ["sound", "sounds"],
    text: ["text file", "text files"],
    other: ["other", "other"],
  };
  const summary = $derived.by(() => {
    if (data.status !== "ready") return undefined;
    const { total_files: n, total_bytes } = data.list;
    return `${n} ${n === 1 ? "file" : "files"}, ${formatBytes(total_bytes)}`;
  });
  const kinds = $derived(
    kindCounts(files)
      .map((k) => `${k.count} ${KIND_WORDS[k.kind][k.count === 1 ? 0 : 1]}`)
      .join(", "),
  );

  function toggle(path: string) {
    const next = new Set(open);
    if (!next.delete(path)) next.add(path);
    open = next;
  }

  let viewOpen = $state(false);
  let viewPath = $state("");
  function view(path: string) {
    viewPath = path;
    viewOpen = true;
  }
</script>

<Section title="Included files" description={summary} class="mod-files">
  {#snippet actions()}
    {#if folders.length > 0 && !filtering}
      {#if open.size > 0}
        <Button size="sm" variant="ghost" onclick={() => (open = new Set())}>Close all</Button>
      {:else}
        <Button size="sm" variant="ghost" onclick={() => (open = new Set(folders))}>Open all</Button>
      {/if}
    {/if}
  {/snippet}
  <div class="files" data-testid="mod-files">
    {#if data.status === "loading"}
      <div aria-busy="true" aria-label="Reading the mod's files"><Skeleton lines={4} /></div>
    {:else if data.status === "error"}
      <p class="qm-muted note" role="note">{data.message}</p>
    {:else if files.length === 0}
      <p class="qm-muted note">This mod's folder has no files.</p>
    {:else}
      {#if files.length >= FILTER_FROM}
        <TextField bind:value={query} label="Filter files" hideLabel placeholder="Filter files" autocomplete="off" spellcheck={false} />
      {/if}
      {#if rows.length === 0}
        <p class="qm-muted note" role="status">No file has "{query.trim()}" in its name.</p>
      {:else}
        <ul class="tree" aria-label={`Files of ${modName}`}>
          {#each rows as r (`${r.type}:${r.path}`)}
            <li>
              {#if r.type === "folder"}
                <button type="button" class="row folder" style:--depth={r.depth} aria-expanded={r.open} aria-label={`Folder ${r.path}`} onclick={() => toggle(r.path)}>
                  <IconChevronRight class={r.open ? "chev open" : "chev"} />
                  <span class="name">{r.name}</span>
                  <span class="meta">{r.files} {r.files === 1 ? "file" : "files"}</span>
                  <span class="size">{formatBytes(r.size)}</span>
                </button>
              {:else}
                <button type="button" class="row file" style:--depth={r.depth} aria-label={`View file ${r.path}`} onclick={() => view(r.path)}>
                  <span class={["kind", r.kind]} aria-hidden="true"></span>
                  <span class="name">{r.name}</span>
                  <span class="size">{formatBytes(r.size)}</span>
                </button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
      <p class="qm-muted note foot">
        {#if filtering && matches.length > rows.length}
          Showing the first {rows.length} of {matches.length} matches.
        {:else if data.list.total_files > files.length}
          Showing the first {files.length} of {data.list.total_files} files. {kinds}.
        {:else}
          {kinds}. Choose a file to view it.
        {/if}
      </p>
    {/if}
  </div>
</Section>

<FileViewDialog bind:open={viewOpen} title={viewPath.split("/").pop() ?? viewPath} files={[{ source: modId, path: viewPath, label: modName }]} />

<style>
  .files {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    min-width: 0;
  }

  .note {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .tree {
    margin: 0;
    padding: 0;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
    list-style: none;
    overflow: hidden;
  }

  li + li {
    border-top: 1px solid var(--qm-border);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
    width: 100%;
    min-height: 28px;
    padding: var(--qm-space-1) var(--qm-space-2);
    padding-left: calc(var(--qm-space-2) + var(--depth, 0) * var(--qm-space-4));
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    font-size: var(--qm-text-12);
    text-align: left;
    cursor: pointer;
  }

  .row:hover {
    background: var(--qm-raised);
  }

  .row:focus-visible {
    outline: none;
    box-shadow: inset var(--qm-focus-ring);
  }

  .folder {
    background: var(--qm-panel);
  }

  .row :global(.chev) {
    flex: none;
    width: 12px;
    height: 12px;
    color: var(--qm-text-muted);
    transition: transform 120ms ease;
  }

  .row :global(.chev.open) {
    transform: rotate(90deg);
  }

  /* A file's kind: a small mark in the place a folder has its arrow. */
  .kind {
    flex: none;
    width: 12px;
    height: 12px;
    display: grid;
    place-items: center;
  }

  .kind::before {
    content: "";
    width: 6px;
    height: 6px;
    border-radius: 1px;
    background: var(--qm-border-strong);
  }

  .kind.code::before {
    background: var(--qm-accent-text);
  }

  .kind.image::before {
    border-radius: 50%;
    background: var(--qm-text-muted);
  }

  .kind.sound::before {
    border-radius: 50%;
    border: 1.5px solid var(--qm-text-muted);
    background: none;
    box-sizing: border-box;
  }

  .name {
    flex: 1;
    min-width: 0;
    font-family: var(--qm-font-mono);
    overflow-wrap: anywhere;
  }

  .folder .name {
    font-weight: 600;
  }

  .meta,
  .size {
    flex: none;
    color: var(--qm-text-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .size {
    min-width: 7ch;
    text-align: right;
  }

  @media (prefers-reduced-motion: reduce) {
    .row :global(.chev) {
      transition: none;
    }
  }
</style>
