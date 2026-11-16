<script lang="ts">
  import { Callout, Dialog, Skeleton } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type AnchorCandidate, type FixDraft, type FixView, type PreviewResult, type RegionOptions, type SiteKey } from "../../../lib/bindings";
  import { edgeChange, lineCheck, moveChange, withinChange, type AnchorChange, type AnchorPick } from "../../../lib/patches/fixes";
  import { fixes } from "../../../lib/stores/fixes.svelte";
  import AnchorPicker from "./AnchorPicker.svelte";

  interface Props {
    open?: boolean;
    /** An Anchor fix that uses its patch's own line (§11.4). */
    fix: FixView | null;
    modNames: Record<string, string>;
    canMutate: boolean;
  }

  let { open = $bindable(false), fix, modNames, canMutate }: Props = $props();

  type Loaded = { file: string; patchId: string; original: string[]; region: RegionOptions | null; candidates: AnchorCandidate[] };
  let loaded = $state<Loaded | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let token = 0;

  const modName = (id: string) => modNames[id] ?? id;
  /** Region mode when the patch the fix placed code next to is known (§11.3). */
  const region = $derived.by(() => {
    const r = loaded?.region;
    if (!r?.b_patch_id) return null;
    return { patchId: r.b_patch_id, modName: r.b_mod_id ? modName(r.b_mod_id) : "", patchName: r.b_patch_id, sites: r.sites };
  });

  async function load(f: FixView) {
    const mine = ++token;
    loaded = null;
    error = null;
    const file = f.target_file;
    const patchId = f.patch?.patch_id;
    if (!file || !patchId) return;
    try {
      const [diff, r] = await Promise.all([
        call(commands.fileDiffGet(file), { expect: ["not_found"] })
          .then((x) => (x.status === "ready" ? x.data : null))
          .catch(() => null),
        call(commands.regionOptions(file, patchId, null), { expect: ["not_found", "conflict", "invalid_input"] }).catch(() => null),
      ]);
      const own = diff?.result.records.find((x) => x.patch_id === patchId)?.regions[0]?.start_line;
      const line = r?.a_sites[0]?.line ?? own ?? 1;
      const candidates = r?.b_patch_id
        ? []
        : await call(commands.anchorCandidates(file, line, 8, null), { expect: ["not_found", "conflict"] }).catch(() => []);
      if (mine !== token) return;
      loaded = { file, patchId, original: diff?.original_lines ?? [], region: r, candidates };
    } catch (e) {
      if (mine === token) error = e instanceof Error ? e.message : String(e);
    }
  }

  const shown = $derived(open && fix ? fix : null);
  $effect(() => {
    if (shown) void load(shown);
  });

  /** The places the fix moves: the one picked, else every one it moved before. */
  function sitesOf(pick: AnchorPick): SiteKey[] {
    if (pick.aSite) return [pick.aSite];
    const list = loaded?.region?.a_sites ?? [];
    return list.length > 1 ? [list[0].key] : [];
  }

  async function changeOf(pick: AnchorPick): Promise<AnchorChange | null> {
    const l = loaded;
    if (!l) return null;
    const sites = sitesOf(pick);
    if (pick.kind === "line") return moveChange(l.patchId, pick.text, pick.position, sites);
    const b = l.region?.b_patch_id;
    if (!b) return null;
    if (pick.kind === "region") return withinChange(l.patchId, b, pick.site, pick.text, pick.nth, pick.position, sites);
    return edgeChange(l.patchId, b, pick.site, pick.edge, await lineCheck(pick.boundary), sites);
  }

  /** The new Anchor fix: the same label and origin as the one it replaces. */
  async function draftOf(pick: AnchorPick): Promise<FixDraft | null> {
    const f = fix;
    const change = await changeOf(pick);
    if (!f || !change) return null;
    return { change, label: f.label, source: f.source, conflict_id: f.provenance.conflict_id ?? null, option_key: null, approved: null };
  }

  async function onpreview(pick: AnchorPick): Promise<PreviewResult | null> {
    const f = fix;
    const draft = await draftOf(pick);
    if (!f || !draft || !loaded) return null;
    return call(commands.simulationPreview({ add_fixes: [draft], remove_fixes: [f.id], focus_file: loaded.file }), {
      expect: ["cancelled", "conflict", "invalid_input"],
    });
  }

  async function onapply(pick: AnchorPick) {
    const f = fix;
    const draft = await draftOf(pick);
    if (!f || !draft) return;
    busy = true;
    try {
      await fixes.replace(f.id, draft);
      open = false;
    } catch {
      // call() already reported it.
    } finally {
      busy = false;
    }
  }
</script>

<Dialog
  bind:open
  title={fix ? `Pick a new line for "${fix.label}"` : "Pick a new line"}
  description={fix?.status_text}
  size="lg"
  initialFocus="content"
  dismissible={!busy}
>
  {#if error}
    <Callout tone="danger" title="Could not load the lines.">{error}</Callout>
  {:else if !loaded}
    <div aria-busy="true" aria-label="Loading the lines"><Skeleton lines={5} /></div>
  {:else}
    <AnchorPicker
      candidates={loaded.candidates}
      {region}
      aSites={loaded.region?.a_sites ?? []}
      conflictId=""
      original={loaded.original}
      {modNames}
      canMutate={canMutate && !busy}
      {onpreview}
      {onapply}
    />
  {/if}
</Dialog>
