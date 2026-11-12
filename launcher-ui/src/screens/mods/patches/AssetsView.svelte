<script lang="ts">
  import { untrack } from "svelte";
  import { Badge, Button, Callout, Dialog, EmptyState, Select, Skeleton } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type AssetChoicePreview, type AssetGroup, type AssetLedger, type AssetSlot, type FactionSideChoice } from "../../../lib/bindings";
  import { VANILLA, choiceDraft, groupSummary, groupTitle, groupWinner, previewLines, slotSources, sourceName, winnerItems } from "../../../lib/patches/assets";
  import { fixes } from "../../../lib/stores/fixes.svelte";
  import { loadOrder } from "../../../lib/stores/loadorder.svelte";

  interface Props {
    modNames: Record<string, string>;
    canMutate: boolean;
  }

  let { modNames, canMutate }: Props = $props();

  type Load<T> = { status: "loading" } | { status: "ready"; data: T } | { status: "error"; message: string };
  let ledger = $state<Load<AssetLedger>>({ status: "loading" });
  let factions = $state<FactionSideChoice[]>([]);
  let seq = 0;
  /** Groups whose files are listed. */
  let open = $state<Record<string, boolean>>({});

  /** A choice waiting for Apply. */
  let pending = $state<{ key: string; title: string; winner: string; preview: Load<AssetChoicePreview> } | null>(null);
  let confirmOpen = $state(false);
  let busy = $state(false);
  let previewSeq = 0;
  /** Remounts the pickers after a cancelled change, so they show the choice in use again. */
  let pickKey = $state(0);

  const modName = (id: string) => modNames[id] ?? id;
  const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

  async function load() {
    const mine = ++seq;
    try {
      const [l, f] = await Promise.all([
        call(commands.assetsLedger(), { expect: ["conflict"] }),
        call(commands.factionChoices(), { expect: ["conflict"] }),
      ]);
      if (mine !== seq) return;
      ledger = l.status === "ready" ? { status: "ready", data: l.data } : { status: "loading" };
      factions = f.status === "ready" ? f.data : [];
    } catch (e) {
      if (mine === seq) ledger = { status: "error", message: message(e) };
    }
  }

  // Reload with every new simulation (a mod change, a new choice).
  $effect(() => {
    if (loadOrder.simulation.status === "ready") void load();
  });

  async function preview(key: string, winner: string) {
    const mine = ++previewSeq;
    if (pending) pending.preview = { status: "loading" };
    try {
      const data = await call(commands.assetChoicePreview(key, winner), { expect: ["conflict", "invalid_input", "not_found"] });
      if (mine === previewSeq && pending) pending.preview = { status: "ready", data };
    } catch (e) {
      if (mine === previewSeq && pending) pending.preview = { status: "error", message: message(e) };
    }
  }

  function pick(key: string, title: string, current: string, winner: string) {
    if (!canMutate || winner === current) return;
    pending = { key, title, winner, preview: { status: "loading" } };
    confirmOpen = true;
    if (winner !== "") void preview(key, winner);
  }

  async function apply() {
    if (!pending || busy) return;
    busy = true;
    try {
      if (pending.winner === "") {
        // Back to "last loaded wins": the choice's fix is undone.
        await call(commands.assetChoiceClear(pending.key));
        await fixes.load().catch(() => {});
      } else {
        await fixes.create(choiceDraft(pending.key, pending.winner));
      }
      confirmOpen = false;
    } catch {
      // call() showed the error.
    } finally {
      busy = false;
    }
  }

  // A closed dialog leaves the pickers on the choice in use.
  $effect(() => {
    if (!confirmOpen) untrack(() => pickKey++);
  });

  async function pickFaction(side: FactionSideChoice, id: string) {
    if (!canMutate || busy) return;
    busy = true;
    try {
      await call(commands.factionPickSet(side.side, id === "" ? null : id));
      await load();
    } catch {
      // call() showed the error.
    } finally {
      busy = false;
      pickKey++;
    }
  }

  const slotTitle = (s: AssetSlot) => `${s.name} (${s.key})`;
  const sources = (g: AssetGroup) => g.claimants.map((c) => c.source);
</script>

<div class="assets-view" data-testid="assets-view">
  {#each factions as side (side.side)}
    <section class="card" aria-label={`Faction for the ${side.label}`} data-testid="faction-picker">
      <h4>{side.candidates.length} factions replace the {side.label}</h4>
      <p class="qm-muted">
        Only one can run. Without a pick the one with the highest load priority runs, so a load order change can switch
        factions and reset class unlocks.
      </p>
      {#key pickKey}
        <Select
          label={`Faction for the ${side.label}`}
          items={[
            { value: "", label: "Highest load priority" },
            ...side.candidates.map((c) => ({ value: c.faction_id, label: `${c.name} (${modName(c.mod_id)})` })),
          ]}
          bind:value={() => side.picked ?? "", (v) => void pickFaction(side, v)}
          disabled={!canMutate || busy}
        />
      {/key}
      <p class="now">
        Runs now: {side.candidates.find((c) => c.faction_id === side.active)?.name ?? "none"}.
      </p>
    </section>
  {/each}

  {#if ledger.status === "loading"}
    <div aria-busy="true" aria-label="Loading overlapping files"><Skeleton lines={4} /></div>
  {:else if ledger.status === "error"}
    <Callout tone="danger" title="Could not load the overlapping files.">
      {ledger.message}
      {#snippet action()}<Button size="sm" onclick={() => load()}>Retry</Button>{/snippet}
    </Callout>
  {:else if ledger.data.groups.length === 0}
    <EmptyState
      title="No overlapping files"
      body="No sound, image or font is replaced by an enabled mod or faction."
    />
  {:else}
    <p class="qm-muted intro">
      Sounds, images and fonts that a mod or faction replaces. When two replace the same file, the one loaded last
      wins unless you pick one here.
    </p>
    {#if !ledger.data.has_game_files}
      <Callout tone="warn" title="The game's file list could not be read.">
        Only files that two mods both ship are listed, and it is not known which of them are the game's own.
      </Callout>
    {/if}
    {#each ledger.data.groups as g (g.key)}
      {@const winner = groupWinner(g)}
      <section class="card" aria-label={groupTitle(g)} data-testid="asset-group">
        <div class="head">
          <div class="title">
            <h4>{groupTitle(g)}</h4>
            <p class="qm-muted">{groupSummary(g, modName)}</p>
          </div>
          {#key pickKey}
            <Select
              label={`Winner for ${groupTitle(g)}`}
              hideLabel
              items={winnerItems(sources(g), g.slots.some((s) => s.game), modName)}
              bind:value={() => g.choice ?? "", (v) => pick(g.key, groupTitle(g), g.choice ?? "", v)}
              disabled={!canMutate}
              class="winner"
            />
          {/key}
        </div>
        <p class="now">
          {#if winner}
            Used now: {sourceName(winner, modName)}{g.choice ? " (your choice)" : ""}.
          {:else}
            Used now: a mix; see the files.
          {/if}
        </p>
        {#each g.notes as n (n.code + n.text)}
          <p class="note" role="note">{n.text}</p>
        {/each}
        <Button size="sm" variant="ghost" aria-expanded={open[g.key] ?? false} onclick={() => (open[g.key] = !(open[g.key] ?? false))}>
          {(open[g.key] ?? false) ? "Hide" : "Show"} the {g.slots.length === 1 ? "file" : `${g.slots.length} files`}
        </Button>
        {#if open[g.key]}
          <ul class="slots">
            {#each g.slots as s (s.key)}
              <li data-testid="asset-slot">
                <div class="slot-row">
                  <span class="name">{s.name}</span>
                  <Badge tone={s.winner === VANILLA ? "neutral" : "info"} icon={false}>{sourceName(s.winner, modName)}</Badge>
                  <span class="qm-muted from">{slotSources(s, modName)}</span>
                  {#if s.claims.length > 1 || s.choice}
                    {#key pickKey}
                      <Select
                        label={`Winner for ${slotTitle(s)}`}
                        hideLabel
                        items={[
                          { value: "", label: g.choice ? "As the group" : "Last loaded wins" },
                          ...winnerItems(s.claims.map((c) => c.source), Boolean(s.game), modName).slice(1),
                        ]}
                        bind:value={() => s.choice ?? "", (v) => pick(s.key, s.name, s.choice ?? "", v)}
                        disabled={!canMutate}
                        class="winner small"
                      />
                    {/key}
                  {/if}
                </div>
                {#each s.notes as n (n.code + n.text)}
                  <p class="note" role="note">{n.text}</p>
                {/each}
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/each}
  {/if}
</div>

{#if pending}
  {@const p = pending}
  <Dialog
    bind:open={confirmOpen}
    title={p.winner === "" ? `Stop choosing for ${p.title}?` : `Use ${sourceName(p.winner, modName).replace(/^The /, "the ")} for ${p.title}?`}
    description={p.winner === ""
      ? "The source loaded last wins again, as before your choice."
      : "The other sources' files for it are not loaded. You can undo this under Fixes."}
    initialFocus="content"
    dismissible={!busy}
  >
    {#if p.winner !== ""}
      {#if p.preview.status === "loading"}
        <div aria-busy="true" aria-label="Checking what changes"><Skeleton lines={3} /></div>
      {:else if p.preview.status === "error"}
        <Callout tone="danger" title="Could not check what this changes.">
          {p.preview.message}
          {#snippet action()}<Button size="sm" onclick={() => preview(p.key, p.winner)}>Check again</Button>{/snippet}
        </Callout>
      {:else}
        <ul class="changes" data-testid="asset-choice-changes">
          {#each previewLines(p.preview.data, modName) as line (line)}<li>{line}</li>{/each}
        </ul>
      {/if}
    {/if}
    {#snippet footer()}
      <Button disabled={busy} onclick={() => (confirmOpen = false)}>Cancel</Button>
      <Button
        variant="primary"
        disabled={busy || (p.winner !== "" && p.preview.status !== "ready")}
        aria-busy={busy}
        onclick={apply}
      >
        {p.winner === "" ? "Stop choosing" : "Use it"}
      </Button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .assets-view {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  p,
  h4 {
    margin: 0;
  }

  h4 {
    font-size: var(--qm-text-13);
    font-weight: 600;
  }

  .intro,
  .now,
  .note,
  .card .qm-muted {
    font-size: var(--qm-text-12);
  }

  .card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--qm-space-2);
    padding: var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-md);
    background: var(--qm-surface-1);
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--qm-space-3);
    width: 100%;
  }

  .title {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .note {
    color: var(--qm-text-muted);
  }

  .slots {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    width: 100%;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .slot-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .name {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  .from {
    flex: 1;
    min-width: 8rem;
  }

  .changes {
    margin: 0;
    padding: 0 0 0 var(--qm-space-4);
  }
</style>
