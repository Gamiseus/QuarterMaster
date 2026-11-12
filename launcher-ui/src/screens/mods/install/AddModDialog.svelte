<script lang="ts">
  import { open as openPicker } from "@tauri-apps/plugin-dialog";
  import { untrack } from "svelte";
  import { Button, Callout, Dialog, Skeleton } from "../../../components/ui";
  import { call, QmCommandError } from "../../../lib/api";
  import { commands, type ArchiveModCandidate } from "../../../lib/bindings";
  import { buildRequests, type Choice } from "../../../lib/install/plan";
  import { game } from "../../../lib/stores/game.svelte";
  import { installs } from "../../../lib/stores/installs.svelte";
  import { toasts } from "../../../lib/stores/toasts.svelte";
  import CandidateList from "./CandidateList.svelte";

  interface Props {
    open?: boolean;
    /** A path dropped on the screen; a new object starts a new inspection. */
    dropped?: { path: string } | null;
    /** Files are being dragged over the window. */
    dragging?: boolean;
  }

  let { open = $bindable(false), dropped = null, dragging = false }: Props = $props();

  type Inspect =
    | { status: "idle" }
    | { status: "loading" }
    | { status: "ready"; candidates: ArchiveModCandidate[] }
    | { status: "error"; message: string; unmanaged: boolean };

  let path = $state<string | null>(null);
  let unmanaged = $state(false);
  let inspect = $state<Inspect>({ status: "idle" });
  let checked = $state<Record<string, boolean>>({});
  let choices = $state<Record<string, Choice>>({});
  let seq = 0;

  const NO_MANIFEST = "No `qm.toml`";

  function reset() {
    seq++;
    path = null;
    unmanaged = false;
    inspect = { status: "idle" };
    checked = {};
    choices = {};
  }

  async function run(target: string, allowUnmanaged: boolean) {
    const mine = ++seq;
    path = target;
    unmanaged = allowUnmanaged;
    inspect = { status: "loading" };
    try {
      const candidates = await call(commands.modImportInspect(target, allowUnmanaged), { expect: ["invalid_input"] });
      if (mine !== seq) return;
      checked = Object.fromEntries(candidates.map((c) => [c.manifest.id, true]));
      choices = Object.fromEntries(candidates.map((c) => [c.manifest.id, "upgrade" as Choice]));
      inspect = { status: "ready", candidates };
    } catch (e) {
      if (mine !== seq) return;
      if (e instanceof QmCommandError && e.kind === "invalid_input") {
        inspect = { status: "error", message: e.message, unmanaged: !allowUnmanaged && e.message.startsWith(NO_MANIFEST) };
      } else {
        // call() already reported it.
        inspect = { status: "idle" };
      }
    }
  }

  $effect(() => {
    if (!open) untrack(reset);
  });

  $effect(() => {
    const d = dropped;
    if (d) untrack(() => void run(d.path, false));
  });

  async function browse(directory: boolean) {
    let picked: string | null;
    try {
      picked = directory
        ? await openPicker({ directory: true })
        : await openPicker({ filters: [{ name: "Mod archive", extensions: ["zip"] }] });
    } catch (e) {
      toasts.error("Could not open the file picker.", e instanceof Error ? e.message : String(e));
      return;
    }
    if (picked) void run(picked, false);
  }

  const candidates = $derived(inspect.status === "ready" ? inspect.candidates : []);
  const chosen = $derived(candidates.filter((c) => checked[c.manifest.id]));
  const reason = $derived(installs.blockReason());
  const canInstall = $derived(reason === null && path !== null && chosen.length > 0);

  function install() {
    if (!canInstall || path === null) return;
    const selected = new Map<string, Choice | null>(
      chosen.map((c) => [c.manifest.id, c.already_installed_version === null ? null : (choices[c.manifest.id] ?? "upgrade")]),
    );
    const byId = new Map(candidates.map((c) => [c.manifest.id, c.manifest.name]));
    const names = (ids: string[]) => ids.map((id) => byId.get(id) ?? id).join(", ");
    installs.run(buildRequests(path, unmanaged, selected), names, () => (open = false)).catch(() => {});
  }
</script>

<Dialog bind:open title="Add mod" size="lg">
  <div class="content">
    <div class={["zone", { dragging }]}>
      <p class="zone-text">Drop a .zip or folder here</p>
      <div class="browse">
        <Button size="sm" onclick={() => browse(false)}>Browse zip</Button>
        <Button size="sm" onclick={() => browse(true)}>Browse folder</Button>
      </div>
    </div>
    {#if path}
      <p class="qm-muted path">{path}</p>
    {/if}
    {#if inspect.status === "loading"}
      <div aria-busy="true" aria-label="Reading the archive"><Skeleton lines={3} /></div>
    {:else if inspect.status === "error" && inspect.unmanaged}
      {@const target = path}
      <Callout tone="warn" title="No QuarterMaster manifest">
        <p>{inspect.message}</p>
        {#snippet action()}
          <Button size="sm" onclick={() => target && run(target, true)}>Import as unmanaged</Button>
        {/snippet}
      </Callout>
    {:else if inspect.status === "error"}
      <Callout tone="danger" title="This archive cannot be installed">
        <p>{inspect.message}</p>
      </Callout>
    {:else if inspect.status === "ready"}
      <CandidateList {candidates} bind:checked bind:choices gameBusy={game.isBusy} />
    {/if}
  </div>
  {#snippet footer()}
    {#if reason && inspect.status === "ready"}
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
