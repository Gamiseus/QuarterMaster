<script lang="ts">
  import { open as openPicker } from "@tauri-apps/plugin-dialog";
  import { untrack } from "svelte";
  import { Button, Callout, Dialog, Skeleton } from "../../../components/ui";
  import { call, QmCommandError } from "../../../lib/api";
  import { commands, type ArchiveModCandidate } from "../../../lib/bindings";
  import { buildRequests, type Choice } from "../../../lib/install/plan";
  import { installs } from "../../../lib/stores/installs.svelte";
  import { toasts } from "../../../lib/stores/toasts.svelte";
  import CandidateList from "./CandidateList.svelte";

  interface Props {
    open?: boolean;
    /** Paths dropped on the screen; a new object starts a new inspection of all of them. */
    dropped?: { paths: string[] } | null;
    /** Files are being dragged over the window. */
    dragging?: boolean;
  }

  let { open = $bindable(false), dropped = null, dragging = false }: Props = $props();

  type Inspect =
    | { status: "loading" }
    | { status: "ready"; candidates: ArchiveModCandidate[] }
    | { status: "error"; message: string; unmanaged: boolean };

  /** One dropped or picked archive or folder, with what was found in it and what is chosen from it. */
  interface Source {
    path: string;
    unmanaged: boolean;
    inspect: Inspect;
    checked: Record<string, boolean>;
    choices: Record<string, Choice>;
  }

  let sources = $state<Source[]>([]);
  let seq = 0;

  const NO_MANIFEST = "No `qm.toml`";

  function reset() {
    seq++;
    sources = [];
  }

  async function inspectOne(index: number, allowUnmanaged: boolean, mine: number) {
    const target = sources[index].path;
    sources[index] = { path: target, unmanaged: allowUnmanaged, inspect: { status: "loading" }, checked: {}, choices: {} };
    let next: Pick<Source, "inspect" | "checked" | "choices">;
    try {
      const candidates = await call(commands.modImportInspect(target, allowUnmanaged), { expect: ["invalid_input"] });
      next = {
        inspect: { status: "ready", candidates },
        checked: Object.fromEntries(candidates.map((c) => [c.manifest.id, true])),
        choices: Object.fromEntries(candidates.map((c) => [c.manifest.id, "upgrade" as Choice])),
      };
    } catch (e) {
      const known = e instanceof QmCommandError && e.kind === "invalid_input";
      // Anything else call() already reported; the entry says so too, so the list never looks complete.
      const message = known ? e.message : "This could not be read.";
      next = { inspect: { status: "error", message, unmanaged: known && !allowUnmanaged && message.startsWith(NO_MANIFEST) }, checked: {}, choices: {} };
    }
    if (mine !== seq || sources[index]?.path !== target) return;
    sources[index] = { ...sources[index], ...next };
  }

  /** Starts over with `paths` (each once), reading them one after another. */
  async function run(paths: string[]) {
    const mine = ++seq;
    const unique = [...new Set(paths)];
    sources = unique.map((p) => ({ path: p, unmanaged: false, inspect: { status: "loading" }, checked: {}, choices: {} }));
    for (let i = 0; i < unique.length; i++) {
      if (mine !== seq) return;
      await inspectOne(i, false, mine);
    }
  }

  $effect(() => {
    if (!open) untrack(reset);
  });

  $effect(() => {
    const d = dropped;
    if (d) untrack(() => void run(d.paths));
  });

  async function browse(directory: boolean) {
    let picked: string | string[] | null;
    try {
      picked = directory
        ? await openPicker({ directory: true, multiple: true })
        : await openPicker({ multiple: true, filters: [{ name: "Mod archive", extensions: ["zip"] }] });
    } catch (e) {
      toasts.error("Could not open the file picker.", e instanceof Error ? e.message : String(e));
      return;
    }
    const paths = picked === null ? [] : Array.isArray(picked) ? picked : [picked];
    if (paths.length > 0) void run(paths);
  }

  const reading = $derived(sources.some((s) => s.inspect.status === "loading"));
  const ready = $derived(sources.some((s) => s.inspect.status === "ready"));
  const chosenCount = $derived(
    sources.reduce((n, s) => n + (s.inspect.status === "ready" ? s.inspect.candidates.filter((c) => s.checked[c.manifest.id]).length : 0), 0),
  );
  const reason = $derived(installs.blockReason());
  const canInstall = $derived(reason === null && !reading && chosenCount > 0);

  function install() {
    if (!canInstall) return;
    const byId = new Map<string, string>();
    const requests = sources.flatMap((s) => {
      if (s.inspect.status !== "ready") return [];
      const chosen = s.inspect.candidates.filter((c) => s.checked[c.manifest.id]);
      for (const c of chosen) byId.set(c.manifest.id, c.manifest.name);
      const selected = new Map<string, Choice | null>(
        chosen.map((c) => [c.manifest.id, c.already_installed_version === null ? null : (s.choices[c.manifest.id] ?? "upgrade")]),
      );
      return buildRequests(s.path, s.unmanaged, selected);
    });
    const names = (ids: string[]) => ids.map((id) => byId.get(id) ?? id).join(", ");
    installs.run(requests, names, () => (open = false)).catch(() => {});
  }
</script>

<Dialog bind:open title="Add mod" size="lg">
  <div class="content">
    <div class={["zone", { dragging }]}>
      <p class="zone-text">Drop .zip files or folders here</p>
      <div class="browse">
        <Button size="sm" onclick={() => browse(false)}>Browse zip</Button>
        <Button size="sm" onclick={() => browse(true)}>Browse folder</Button>
      </div>
    </div>
    {#each sources as src, i (src.path)}
      <div class="source">
        <p class="qm-muted path">{src.path}</p>
        {#if src.inspect.status === "loading"}
          <div aria-busy="true" aria-label="Reading the archive"><Skeleton lines={3} /></div>
        {:else if src.inspect.status === "error" && src.inspect.unmanaged}
          <Callout tone="warn" title="No QuarterMaster manifest">
            <p>{src.inspect.message}</p>
            {#snippet action()}
              <Button size="sm" onclick={() => inspectOne(i, true, seq)}>Import as unmanaged</Button>
            {/snippet}
          </Callout>
        {:else if src.inspect.status === "error"}
          <Callout tone="danger" title="This archive cannot be installed">
            <p>{src.inspect.message}</p>
          </Callout>
        {:else}
          <CandidateList candidates={src.inspect.candidates} bind:checked={src.checked} bind:choices={src.choices} />
        {/if}
      </div>
    {/each}
  </div>
  {#snippet footer()}
    {#if reason && ready}
      <p class="qm-muted reason">{reason}</p>
    {/if}
    <Button variant="ghost" onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" disabled={!canInstall} onclick={install}>Install</Button>
  {/snippet}
</Dialog>

<style>
  .content {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .zone {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--qm-space-3);
    padding: var(--qm-space-5);
    border: 1px dashed var(--qm-border-strong);
    border-radius: var(--qm-radius-md);
    transition: border-color var(--qm-dur-fast) var(--qm-ease);
  }

  .zone.dragging {
    border-color: var(--qm-accent);
    background: color-mix(in srgb, var(--qm-accent) 8%, transparent);
  }

  .zone-text {
    margin: 0;
    font-size: var(--qm-text-14);
  }

  .browse {
    display: flex;
    gap: var(--qm-space-2);
  }

  .source {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .path {
    margin: 0;
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    overflow-wrap: anywhere;
  }

  .reason {
    margin: 0 auto 0 0;
    align-self: center;
    font-size: var(--qm-text-12);
  }
</style>
