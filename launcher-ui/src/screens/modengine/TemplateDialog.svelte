<script lang="ts">
  import { Badge, Button, Callout, Dialog, Skeleton, TextField } from "../../components/ui";
  import { QmCommandError, call } from "../../lib/api";
  import { commands, type TemplateAddPreview, type TemplateChangeKind, type TemplateFileView, type TemplateInfo } from "../../lib/bindings";
  import { openExternal } from "../../lib/external";
  import { engine } from "../../lib/stores/engine.svelte";

  /** The wiki's parts, as the catalog groups templates. */
  const PARTS: Record<number, string> = {
    1: "What a mod is",
    2: "Art and sound",
    3: "Factions",
    4: "Changing game code",
    5: "Your own Lua files",
    6: "Settings, keybinds and menus",
    7: "Working with other mods",
    8: "Game reference",
    9: "Tools",
    10: "Testing and debugging",
    11: "Packaging and publishing",
    12: "The Mod Engine",
  };
  /** Templates that are a whole mod: they make a new mod but add nothing to one. */
  const MOD_ONLY = new Set(["empty", "compat-patch"]);
  const CHANGE: Record<TemplateChangeKind, { tone: "ok" | "info" | "neutral"; text: string }> = {
    add: { tone: "ok", text: "New" },
    merge: { tone: "info", text: "Added to" },
    skip: { tone: "neutral", text: "Kept" },
  };

  const mode = $derived(engine.templateDialog ?? "new");
  const modName = $derived(engine.projects?.projects.find((p) => p.folder === engine.folder)?.name ?? engine.folder ?? "");

  let query = $state("");
  let pickedId = $state<string | null>(null);
  let files = $state<TemplateFileView[] | null>(null);
  let shownFile = $state<string | null>(null);
  let addPreview = $state<TemplateAddPreview | null>(null);
  let name = $state("");
  let modId = $state("");
  let idEdited = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);
  const previews = new Map<string, TemplateFileView[]>();

  const offered = $derived(engine.templates.filter((t) => mode === "new" || !MOD_ONLY.has(t.id)));
  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return offered;
    return offered.filter((t) =>
      [t.name, t.summary, t.section, t.id, ...t.aliases].some((s) => s.toLowerCase().includes(q)),
    );
  });
  const groups = $derived.by(() => {
    const out: { part: number; items: TemplateInfo[] }[] = [];
    for (const t of shown) {
      const last = out[out.length - 1];
      if (last?.part === t.part) last.items.push(t);
      else out.push({ part: t.part, items: [t] });
    }
    return out;
  });
  const picked = $derived(engine.templates.find((t) => t.id === pickedId) ?? null);
  const file = $derived(files?.find((f) => f.path === shownFile) ?? null);

  // New mod: the folder the name makes, and what is wrong with the name or id.
  const folder = $derived([...name.trim()].filter((c) => /[A-Za-z0-9_-]/.test(c)).join(""));
  const taken = $derived(folder !== "" && (engine.projects?.projects.some((p) => p.folder.toLowerCase() === folder.toLowerCase()) ?? false));
  const idProblem = $derived(
    modId === "" ? null : /^[a-z][a-z0-9_-]*$/.test(modId) ? null : "An id starts with a lower-case letter and has only a to z, 0 to 9, _ and -.",
  );
  const canCreate = $derived(picked !== null && folder !== "" && modId !== "" && !taken && !idProblem && !busy);
  const canAdd = $derived(picked !== null && addPreview !== null && addPreview.blocked.length === 0 && !busy);

  /** An id from the name: lower case, words joined by _. */
  function idFrom(text: string): string {
    const id = text
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "_")
      .replace(/^_+|_+$/g, "");
    return id && !/^[a-z]/.test(id) ? `mod_${id}` : id;
  }

  $effect(() => {
    if (engine.templateDialog === null) return;
    engine.loadTemplates().catch(() => {});
  });

  // Open: start from the first template on offer, with an empty form.
  $effect(() => {
    if (engine.templateDialog === null) return;
    query = "";
    name = "";
    modId = "";
    idEdited = false;
    error = null;
    pickedId = null;
  });

  $effect(() => {
    if (engine.templateDialog !== null && pickedId === null && offered.length > 0) pickedId = offered[0].id;
  });

  // The picked template's files, and in add mode what adding it would change.
  $effect(() => {
    const id = pickedId;
    if (!id || engine.templateDialog === null) return;
    error = null;
    const cached = previews.get(id);
    files = cached ?? null;
    if (cached) shownFile = pickOpen(id, cached);
    if (!cached) {
      call(commands.templatePreview(id))
        .then((f) => {
          previews.set(id, f);
          if (pickedId !== id) return;
          files = f;
          shownFile = pickOpen(id, f);
        })
        .catch(() => {});
    }
    addPreview = null;
    const target = engine.folder;
    if (engine.templateDialog === "add" && target) {
      call(commands.templateAddPreview(target, id))
        .then((p) => {
          if (pickedId === id) addPreview = p;
        })
        .catch(() => {});
    }
  });

  function pickOpen(id: string, f: TemplateFileView[]): string | null {
    const open = engine.templates.find((t) => t.id === id)?.open;
    return f.find((x) => x.path === open)?.path ?? f[0]?.path ?? null;
  }

  function onName(e: Event) {
    const v = (e.currentTarget as HTMLInputElement).value;
    if (!idEdited) modId = idFrom(v);
  }

  function onId() {
    idEdited = true;
  }

  async function submit() {
    if (!picked) return;
    busy = true;
    error = null;
    try {
      const done = mode === "new" ? await engine.createFromTemplate(picked, name.trim(), modId) : await engine.addExample(picked);
      if (done) engine.templateDialog = null;
    } catch (e) {
      if (e instanceof QmCommandError) error = e.message;
    } finally {
      busy = false;
    }
  }

  function isImage(path: string): boolean {
    return /\.(png|jpe?g|bmp|gif|webp|tga)$/i.test(path);
  }

  function size(bytes: number): string {
    return bytes < 1024 ? `${bytes} bytes` : `${Math.round(bytes / 1024)} KB`;
  }
</script>

<Dialog
  bind:open={() => engine.templateDialog !== null, (v) => !v && (engine.templateDialog = null)}
  title={mode === "new" ? "New mod from a template" : `Add an example to ${modName}`}
  description={mode === "new"
    ? "Each template is a small working mod for one part of the modding kit, with comments that explain it."
    : "The example's files go into the mod; its patches, settings, keybinds and faction sections join the mod's own. Nothing of the mod's is overwritten."}
  size="xl"
  explicitClose
  dismissible={!busy}
>
  <div class="gallery">
    <div class="pick">
      <TextField label="Search the templates" hideLabel placeholder="Search: sound, keybind, faction..." bind:value={query} />
      {#if engine.templates.length === 0}
        <Skeleton lines={8} />
      {:else if shown.length === 0}
        <p class="qm-muted none">No template matches "{query}".</p>
      {:else}
        <div class="list" aria-label="Templates">
          {#each groups as g (g.part)}
            {@const label = `${g.part}. ${PARTS[g.part] ?? "Other"}`}
            <p class="qm-stencil part" aria-hidden="true">{label}</p>
            <ul aria-label={label}>
              {#each g.items as t (t.id)}
                <li>
                  <button
                    type="button"
                    class={["item", { current: t.id === pickedId }]}
                    aria-current={t.id === pickedId ? "true" : undefined}
                    onclick={() => (pickedId = t.id)}
                  >
                    <span class="name">{t.name}</span>
                    <span class="summary qm-muted">{t.summary}</span>
                  </button>
                </li>
              {/each}
            </ul>
          {/each}
        </div>
      {/if}
    </div>

    <div class="detail" aria-live="polite">
      {#if picked}
        <div class="about">
          <h3 class="title">{picked.name}</h3>
          <p class="summary-full">{picked.summary}</p>
          <p class="wiki qm-muted">
            The wiki explains it under "{picked.section}".
            <button type="button" class="link" onclick={() => openExternal(picked.wiki_url)}>Read it</button>
          </p>
          {#if picked.try_next.length > 0}
            <div class="next">
              <p class="label qm-muted">Then try</p>
              <ul>
                {#each picked.try_next as n (n)}<li>{n}</li>{/each}
              </ul>
            </div>
          {/if}
        </div>

        <div class="files">
          <div class="file-tabs" role="group" aria-label="Files of the template">
            {#each files ?? [] as f (f.path)}
              <button type="button" aria-pressed={f.path === shownFile} onclick={() => (shownFile = f.path)}>{f.path}</button>
            {/each}
          </div>
          {#if files === null}
            <Skeleton lines={6} />
          {:else if file}
            {#if file.text === null}
              <p class="qm-muted binary">
                {file.path} is a placeholder {isImage(file.path) ? "image" : "sound"} ({size(file.size_bytes)}): swap
                it for your own.
              </p>
            {:else}
              <pre class="code" aria-label={`${file.path}, as the template has it`}>{file.text}</pre>
            {/if}
          {/if}
        </div>
      {/if}
    </div>
  </div>

  <div class="form">
    {#if mode === "new"}
      <div class="fields">
        <TextField label="Mod name" placeholder="Field Radio" bind:value={name} oninput={onName} data-autofocus="" />
        <TextField label="Mod id" placeholder="field_radio" bind:value={modId} oninput={onId} spellcheck="false" />
      </div>
      <p class={["hint", { "qm-muted": !taken && !idProblem, problem: taken || idProblem }]}>
        {#if taken}
          The workspace already has a folder named {folder}.
        {:else if idProblem}
          {idProblem}
        {:else if folder}
          The mod goes in the workspace folder {folder}. The template's example names become its own.
        {:else}
          The name is what players see; the id is what other mods and the launcher use, and does not change later.
        {/if}
      </p>
    {:else if picked}
      {#if addPreview === null}
        <Skeleton lines={2} />
      {:else}
        {#if addPreview.blocked.length > 0}
          <Callout tone="warn" title="This example cannot be added to {modName}">
            <ul class="blocked">
              {#each addPreview.blocked as b (b)}<li>{b}</li>{/each}
            </ul>
          </Callout>
        {/if}
        {#if addPreview.changes.length > 0}
          <ul class="changes" aria-label="What adding it changes">
            {#each addPreview.changes as c (c.path)}
              <li>
                <Badge tone={CHANGE[c.kind].tone} icon={false}>{CHANGE[c.kind].text}</Badge>
                <span class="path">{c.path}</span>
                <span class="qm-muted">{c.detail}</span>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
    {/if}
    {#if error}<p class="hint problem" role="alert">{error}</p>{/if}
  </div>

  {#snippet footer()}
    <Button variant="ghost" disabled={busy} onclick={() => (engine.templateDialog = null)}>Cancel</Button>
    {#if mode === "new"}
      <Button variant="primary" disabled={!canCreate} aria-busy={busy} onclick={submit}>Create mod</Button>
    {:else}
      <Button variant="primary" disabled={!canAdd} aria-busy={busy} onclick={submit}>Add to {modName}</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .gallery {
    display: grid;
    flex: 1;
    grid-template-columns: minmax(16rem, 20rem) minmax(0, 1fr);
    gap: var(--qm-space-4);
    min-height: 0;
  }

  .pick {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    min-height: 0;
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 0;
    overflow: auto;
    padding-right: var(--qm-space-1);
  }

  .list ul {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .part {
    margin: var(--qm-space-3) 0 var(--qm-space-1);
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }

  .part:first-child {
    margin-top: 0;
  }

  .item {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: var(--qm-space-2);
    border: 1px solid transparent;
    border-radius: var(--qm-radius-sm);
    background: none;
    color: var(--qm-text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .item:hover {
    background: var(--qm-raised);
  }

  .item.current {
    border-color: var(--qm-accent);
    background: color-mix(in srgb, var(--qm-accent) 10%, transparent);
  }

  .name {
    font-weight: 600;
  }

  .summary {
    overflow: hidden;
    font-size: var(--qm-text-12);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .none {
    margin: 0;
    font-size: var(--qm-text-13);
  }

  .detail {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
    min-width: 0;
    min-height: 0;
    padding-left: var(--qm-space-4);
    border-left: 1px solid var(--qm-border);
  }

  .about {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .title {
    margin: 0;
    font-size: var(--qm-text-16);
  }

  .summary-full,
  .wiki {
    margin: 0;
    font-size: var(--qm-text-13);
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

  .next .label {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .next ul {
    margin: var(--qm-space-1) 0 0;
    padding-left: var(--qm-space-5);
    font-size: var(--qm-text-13);
  }

  .files {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--qm-space-2);
    min-height: 0;
  }

  .file-tabs {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-1);
  }

  .file-tabs button {
    padding: 2px var(--qm-space-2);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: none;
    color: var(--qm-text-muted);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    cursor: pointer;
  }

  .file-tabs button[aria-pressed="true"] {
    border-color: var(--qm-accent);
    background: color-mix(in srgb, var(--qm-accent) 14%, transparent);
    color: var(--qm-text);
  }

  .code {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: var(--qm-space-3);
    overflow: auto;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    line-height: 1.5;
    tab-size: 4;
  }

  .binary {
    margin: 0;
    font-size: var(--qm-text-13);
  }

  .form {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin-top: var(--qm-space-3);
    padding-top: var(--qm-space-3);
    border-top: 1px solid var(--qm-border);
  }

  .fields {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--qm-space-3);
  }

  .hint {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .problem {
    color: var(--qm-danger);
  }

  .blocked {
    margin: 0;
    padding-left: var(--qm-space-5);
  }

  .changes {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    max-height: 7rem;
    margin: 0;
    padding: 0;
    overflow: auto;
    list-style: none;
    font-size: var(--qm-text-13);
  }

  .changes li {
    display: flex;
    align-items: baseline;
    gap: var(--qm-space-2);
  }

  .path {
    font-family: var(--qm-font-mono);
  }
</style>
