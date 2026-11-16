<script lang="ts">
  import { call } from "../lib/api";
  import { commands, type FileView } from "../lib/bindings";
  import { formatBytes } from "../lib/saves/format";
  import LuaCode from "../screens/mods/patches/LuaCode.svelte";
  import { Button, Dialog, Skeleton } from "./ui";

  /** One version of the file: the game's (`vanilla`) or a mod's. */
  export interface ViewSource {
    source: string;
    path: string;
    label: string;
  }

  interface Props {
    open?: boolean;
    title: string;
    files: ViewSource[];
  }

  let { open = $bindable(false), title, files }: Props = $props();

  type Entry = { status: "loading" } | { status: "ready"; data: FileView } | { status: "error"; message: string };
  let at = $state(0);
  let loaded = $state<Record<string, Entry>>({});
  const keyOf = (f: ViewSource) => `${f.source}|${f.path}`;
  const current = $derived(files[Math.min(at, files.length - 1)] ?? null);
  const entry = $derived(current ? (loaded[keyOf(current)] ?? { status: "loading" as const }) : null);

  async function load(f: ViewSource) {
    const key = keyOf(f);
    if (loaded[key]) return;
    loaded[key] = { status: "loading" };
    try {
      const data = await call(commands.fileView(f.source, f.path), { expect: ["not_found", "invalid_input"] });
      loaded[key] = { status: "ready", data };
    } catch (e) {
      loaded[key] = { status: "error", message: e instanceof Error ? e.message : String(e) };
    }
  }

  // A newly opened dialog starts on its first file with nothing cached from the last one.
  $effect(() => {
    if (!open) return;
    void files;
    at = 0;
    loaded = {};
  });

  $effect(() => {
    if (open && current) void load(current);
  });

  /** Written text (a readme, notes): its lines wrap. Code and data keep their lines. */
  const isProse = (path: string) => /\.(md|txt)$/i.test(path);
</script>

<Dialog bind:open {title} size="lg" initialFocus="content">
  <div class="file-view" data-testid="file-view">
    {#if files.length > 1}
      <div class="tabs" role="group" aria-label="Version">
        {#each files as f, i (keyOf(f))}
          <Button size="sm" variant={i === at ? "secondary" : "ghost"} aria-pressed={i === at} onclick={() => (at = i)}>{f.label}</Button>
        {/each}
      </div>
    {/if}
    {#if current && entry}
      <p class="qm-muted path">
        {current.label}: {current.path}{entry.status === "ready" ? `, ${formatBytes(entry.data.size_bytes)}` : ""}
      </p>
      {#if entry.status === "loading"}
        <div aria-busy="true" aria-label="Loading the file"><Skeleton lines={5} /></div>
      {:else if entry.status === "error"}
        <p class="missing" role="note">{entry.message}</p>
      {:else if entry.data.kind === "text"}
        <div class="code">
          {#if current.path.toLowerCase().endsWith(".lua")}
            <LuaCode code={entry.data.text ?? ""} label={`${current.label}: ${current.path}`} />
          {:else}
            <pre class={{ prose: isProse(current.path) }} aria-label={`${current.label}: ${current.path}`}>{entry.data.text ?? ""}</pre>
          {/if}
        </div>
      {:else if entry.data.kind === "image" && entry.data.data_url}
        <div class="image"><img src={entry.data.data_url} alt={`${current.label}: ${current.path}`} /></div>
      {:else if entry.data.kind === "audio" && entry.data.data_url}
        <audio controls src={entry.data.data_url} aria-label={`${current.label}: ${current.path}`}></audio>
      {:else}
        <p class="missing" role="note">The launcher cannot show this kind of file.</p>
      {/if}
    {/if}
  </div>
  {#snippet footer()}
    <Button onclick={() => (open = false)}>Close</Button>
  {/snippet}
</Dialog>

<style>
  .file-view {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    min-width: 0;
  }

  .tabs {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-1);
  }

  .path {
    margin: 0;
    font-size: var(--qm-text-12);
    overflow-wrap: anywhere;
  }

  .missing {
    margin: 0;
    font-size: var(--qm-text-13);
  }

  .code {
    max-height: 60vh;
    padding: var(--qm-space-2);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-panel);
    overflow: auto;
  }

  .code pre {
    margin: 0;
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    white-space: pre;
  }

  .code pre.prose {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  /* Game art is small pixel art: shown larger, with hard pixels, on a neutral ground. */
  .image {
    display: grid;
    place-items: center;
    max-height: 60vh;
    padding: var(--qm-space-4);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-raised);
    overflow: auto;
  }

  .image img {
    min-width: min(100%, 96px);
    max-width: 100%;
    image-rendering: pixelated;
  }

  audio {
    width: 100%;
  }
</style>
