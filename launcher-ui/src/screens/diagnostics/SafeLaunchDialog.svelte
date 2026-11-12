<script lang="ts">
  import { untrack } from "svelte";
  import { Button, Callout, Dialog, Skeleton } from "../../components/ui";
  import { call } from "../../lib/api";
  import {
    commands,
    type BisectView,
    type GameStatus,
    type PreviewResult,
    type RunEntry,
    type TrialResult,
    type TrialSuspect,
    type UndoPreview,
  } from "../../lib/bindings";
  import {
    SEARCH_STEPS,
    finishedTest,
    foundCandidate,
    keepOffLabel,
    openSearch,
    readTest,
    remainingLabels,
    searchEndText,
    testCount,
    testSteps,
  } from "../../lib/diagnostics/safelaunch";
  import { app } from "../../lib/stores/app.svelte";
  import { diagnostics } from "../../lib/stores/diagnostics.svelte";
  import { fixes } from "../../lib/stores/fixes.svelte";
  import { game } from "../../lib/stores/game.svelte";
  import { loadOrder } from "../../lib/stores/loadorder.svelte";
  import { mods } from "../../lib/stores/mods.svelte";
  import { profiles } from "../../lib/stores/profiles.svelte";
  import { saves } from "../../lib/stores/saves.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";
  import ImpactNotes from "../mods/patches/ImpactNotes.svelte";

  interface Props {
    open?: boolean;
    /** The run that crashed. */
    run: RunEntry | null;
  }

  let { open = $bindable(false), run }: Props = $props();

  /** `search` is the bisect (D68 step 2); the others are the single suspect test. */
  type Phase = "intro" | "running" | "ask" | "result" | "search";
  type Verdict = "fine" | "crashed" | "stopped" | "failed";

  const SAVE_NOTE = "Your save is backed up before each test; a test without a mod can save a game without that mod's data.";
  /** Suspects named in the search before the rest are counted. */
  const SHOWN_SUSPECTS = 6;

  let phase = $state<Phase>("intro");
  /** undefined while it loads. */
  let suspect = $state<TrialSuspect | null | undefined>(undefined);
  let preview = $state<PreviewResult | null>(null);
  let previewError = $state<string | null>(null);
  let launchError = $state<string | null>(null);
  let launching = $state(false);
  let test = $state<RunEntry | null>(null);
  let verdict = $state<Verdict | null>(null);
  let failReason = $state<string | null>(null);
  /** The test ran, but cleaning up after it failed. */
  let cleanupNote = $state<string | null>(null);
  let applying = $state(false);
  let applied = $state(false);
  let undo = $state<UndoPreview | null>(null);
  let restoreAsked = $state(false);
  let restored = $state(false);
  /** The search, its next test's preview, and the preview of keeping the found cause off. */
  let search = $state<BisectView | null>(null);
  let searchNote = $state<string | null>(null);
  let searchPreview = $state<PreviewResult | null>(null);
  let searchError = $state<string | null>(null);
  let searchBusy = $state(false);
  let culpritPreview = $state<PreviewResult | null>(null);
  let token = 0;
  /** The game status a finished search test was read for, so each run is read once. */
  let readFor: GameStatus | null = null;

  const profileId = $derived(profiles.activeSummary?.id ?? profiles.active?.id ?? null);
  const modNames = $derived(mods.list.status === "ready" ? Object.fromEntries(mods.list.data.map((m) => [m.id, m.name])) : {});
  const found = $derived(search ? foundCandidate(search) : null);
  /** What the result's fix buttons act on: the tested suspect, or the cause the search found. */
  const target = $derived(phase === "search" ? (found?.suspect ?? null) : (suspect ?? null));
  const targetPreview = $derived(phase === "search" ? culpritPreview : preview);
  /** The suspect patch by its name in the impact notes. */
  const name = (patchId: string) => (target?.kind === "patch" && target.id === patchId ? target.name : patchId);
  const crashMessage = $derived(run?.message ?? null);
  /** A load error is not a crash: Health says the game did not load everything. */
  const heading = $derived.by(() => {
    const lead = run?.outcome === "load_error" ? "The last run did not load everything" : "The last run crashed";
    return crashMessage ? `${lead}: ${crashMessage}` : `${lead}.`;
  });
  /** Why nothing is named: a native crash (or a load error) leaves no crash report. */
  const nothingNamed = $derived(
    run && run.crashes.length === 0
      ? "The game left no crash report naming a fix, patch or mod."
      : "The crash report does not name a fix, patch or mod.",
  );
  const canLaunch = $derived(
    app.startupDone && !game.isBusy && !launching && suspect != null && preview !== null && !preview.impact.breaks_file,
  );
  const canSearch = $derived(app.startupDone && !game.isBusy && !searchBusy && run !== null);
  const searchRunning = $derived(search?.running === true);
  const canLaunchSearch = $derived(
    app.startupDone && !game.isBusy && !launching && !searchBusy && search !== null && search.end === null && !search.running && !search.ask &&
      searchPreview !== null && !searchPreview.impact.breaks_file,
  );
  const backup = $derived.by(() => {
    if (phase === "search") {
      return search?.save_slot_id && search.save_backup_id ? { slot: search.save_slot_id, id: search.save_backup_id } : null;
    }
    return test?.trial?.save_slot_id && test.trial.save_backup_id ? { slot: test.trial.save_slot_id, id: test.trial.save_backup_id } : null;
  });
  const shownSuspects = $derived(search ? remainingLabels(search) : []);

  function reset() {
    phase = "intro";
    suspect = undefined;
    preview = null;
    previewError = null;
    launchError = null;
    launching = false;
    test = null;
    verdict = null;
    failReason = null;
    cleanupNote = null;
    applying = false;
    applied = false;
    undo = null;
    restoreAsked = false;
    restored = false;
    search = null;
    searchNote = null;
    searchPreview = null;
    searchError = null;
    searchBusy = false;
    culpritPreview = null;
    readFor = null;
  }

  const capitalize = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
  /** A fix by its label, as the undo lists name them. */
  const fixLabel = (id: string) => fixes.list.find((f) => f.id === id)?.label ?? id;
  const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

  // Each opening picks up a running search, or reads the suspect and previews the test again.
  $effect(() => {
    if (!open || !run) return;
    const mine = ++token;
    const crash = run.crashes[0] ?? null;
    reset();
    const pid = untrack(() => profileId);
    diagnostics
      .bisectGet()
      .then((status) => {
        if (mine !== token) return;
        const going = openSearch(status, pid);
        if (going) {
          phase = "search";
          return showSearch(going);
        }
        searchNote = status.note;
        return readSuspect(mine, crash);
      })
      .catch(() => {
        if (mine === token) return readSuspect(mine, crash);
      });
  });

  async function readSuspect(mine: number, crash: string | null) {
    if (crash === null) {
      suspect = null;
      return;
    }
    try {
      const s = await diagnostics.suspect(crash);
      if (mine !== token) return;
      suspect = s;
      if (!s) return;
      const p = await diagnostics.trialPreview(s.changes);
      if (mine === token) preview = p;
    } catch (e) {
      if (mine !== token) return;
      if (suspect === undefined) suspect = null;
      previewError = errorText(e);
    }
  }

  /** Previews the suspect's test again, after a preview that failed. */
  async function retryPreview() {
    const s = suspect;
    if (!s) return;
    const mine = token;
    previewError = null;
    preview = null;
    try {
      const p = await diagnostics.trialPreview(s.changes);
      if (mine === token) preview = p;
    } catch (e) {
      if (mine === token) previewError = errorText(e);
    }
  }

  async function launch() {
    if (!suspect || !canLaunch) return;
    launching = true;
    launchError = null;
    try {
      await game.launchTrial({ changes: suspect.changes, label: suspect.label });
      phase = "running";
    } catch (e) {
      launchError = errorText(e);
    } finally {
      launching = false;
    }
  }

  // The test ends when the game is no longer busy: read its run from the log.
  $effect(() => {
    if (phase !== "running" || game.isBusy) return;
    const stage = game.status.stage;
    void finish(stage.stage === "failed" ? stage.reason : null);
  });

  async function finish(failed: string | null) {
    const mine = token;
    const id = profileId;
    let found: RunEntry | null = null;
    if (id) {
      try {
        found = finishedTest(await diagnostics.runs(id), run?.run_id ?? null);
      } catch {
        // call() already showed the error.
      }
    }
    if (mine !== token || phase !== "running") return;
    test = found;
    // A run with an outcome ran, even when cleaning up after it failed.
    const ran = found !== null && found.outcome !== "unknown";
    if (failed !== null && !ran) {
      failReason = failed;
      verdict = "failed";
      phase = "result";
      return;
    }
    cleanupNote = failed;
    const reading = found ? readTest(found, run?.outcome ?? null) : "ask";
    if (reading === "ask") {
      phase = "ask";
    } else {
      verdict = reading;
      phase = "result";
    }
  }

  function answer(v: Verdict) {
    verdict = v;
    phase = "result";
  }

  // ---- The search (D68 step 2) ----

  /** Shows `v` and loads what its next step needs: the next test's preview, or the found cause's. */
  async function showSearch(v: BisectView) {
    search = v;
    searchError = null;
    if (v.end === null && !v.running && !v.ask) await checkNext();
    else if (v.end === "found") await previewCulprit();
  }

  async function startSearch() {
    if (!run || !canSearch) return;
    const mine = token;
    searchBusy = true;
    searchError = null;
    try {
      const v = await diagnostics.bisectStart(run.run_id);
      if (mine !== token) return;
      phase = "search";
      applied = false;
      undo = null;
      restoreAsked = false;
      restored = false;
      await showSearch(v);
    } catch (e) {
      if (mine === token) launchError = errorText(e);
    } finally {
      searchBusy = false;
    }
  }

  /** Picks the next test and previews it; the service skips tests that would break a file. */
  async function checkNext() {
    const mine = token;
    searchBusy = true;
    searchPreview = null;
    searchError = null;
    try {
      const r = await diagnostics.bisectPreview();
      if (mine !== token) return;
      search = r.bisect;
      searchPreview = r.preview;
      if (r.bisect.end === "found") await previewCulprit();
    } catch (e) {
      if (mine === token) searchError = errorText(e);
    } finally {
      if (mine === token) searchBusy = false;
    }
  }

  async function previewCulprit() {
    const mine = token;
    const c = search ? foundCandidate(search) : null;
    if (!c) return;
    culpritPreview = null;
    searchError = null;
    try {
      const p = await diagnostics.trialPreview(c.suspect.changes);
      if (mine === token) culpritPreview = p;
    } catch (e) {
      if (mine === token) searchError = errorText(e);
    }
  }

  async function launchSearchTest() {
    if (!canLaunchSearch) return;
    const mine = token;
    launching = true;
    searchError = null;
    try {
      await game.launchBisect();
      const status = await diagnostics.bisectGet();
      if (mine === token && status.bisect) search = status.bisect;
    } catch (e) {
      if (mine !== token) return;
      searchError = errorText(e);
      // A refused split is skipped by the service: pick the next one.
      const status = await diagnostics.bisectGet().catch(() => null);
      if (mine === token && status?.bisect) await showSearch(status.bisect);
    } finally {
      launching = false;
    }
  }

  // A search test ends when the game is no longer busy: the service has read its run.
  $effect(() => {
    const st = game.status;
    if (phase !== "search" || !searchRunning || game.isBusy) return;
    untrack(() => {
      if (readFor === st) return;
      readFor = st;
      void refreshSearch();
    });
  });

  async function refreshSearch() {
    const mine = token;
    try {
      const status = await diagnostics.bisectGet();
      if (mine !== token) return;
      if (status.bisect) await showSearch(status.bisect);
    } catch (e) {
      if (mine === token) searchError = errorText(e);
    }
  }

  async function answerSearch(result: TrialResult) {
    const mine = token;
    searchBusy = true;
    try {
      const v = await diagnostics.bisectAnswer(result);
      if (mine === token) await showSearch(v);
    } catch {
      // call() already showed the error.
    } finally {
      if (mine === token) searchBusy = false;
    }
  }

  async function stopSearch() {
    const mine = token;
    searchBusy = true;
    try {
      const v = await diagnostics.bisectStop();
      if (mine === token) search = v;
    } catch {
      // call() already showed the error.
    } finally {
      if (mine === token) searchBusy = false;
    }
  }

  // ---- The fix path, for the tested suspect or the found cause ----

  async function keepOff() {
    const s = target;
    if (!s) return;
    applying = true;
    try {
      if (s.kind === "fix") await fixes.setPaused(s.id, true);
      else if (s.kind === "patch") await loadOrder.whileChanging(() => call(commands.patchSetEnabled(s.id, false)));
      else await loadOrder.whileChanging(() => call(commands.modSetEnabled(s.id, false)));
      applied = true;
      toasts.info(`Kept ${s.label} off in ${profiles.active?.name ?? "the profile"}.`);
    } catch {
      // call() already showed the error.
    } finally {
      applying = false;
    }
  }

  async function askUndo() {
    if (!target) return;
    applying = true;
    try {
      undo = await fixes.undoPreview(target.id);
    } catch (e) {
      toasts.error(errorText(e));
    } finally {
      applying = false;
    }
  }

  async function confirmUndo() {
    const s = target;
    if (!s || !undo) return;
    applying = true;
    try {
      await fixes.undo(s.id, undo.also_undoes.length > 0 || undo.asks.length > 0);
      applied = true;
      undo = null;
      toasts.info(`Undid ${s.label}.`);
    } catch {
      // The store showed the error.
    } finally {
      applying = false;
    }
  }

  async function restore() {
    if (!backup) return;
    applying = true;
    try {
      await saves.restore(backup.slot, backup.id);
      restored = true;
      restoreAsked = false;
      toasts.info(phase === "search" ? "The save is back to how it was before the tests." : "The save is back to how it was before the test.");
    } catch {
      // call() already showed the error.
    } finally {
      applying = false;
    }
  }
</script>

{#snippet fixPath(s: TrialSuspect, impact: PreviewResult | null)}
  {#if !applied}
    {#if impact}
      <ImpactNotes impact={impact.impact} {modNames} {name} />
    {:else}
      <div aria-busy="true" aria-label="Checking the change"><Skeleton lines={2} /></div>
    {/if}
    {#if undo}
      <p>Undoing the fix does this:</p>
      <ImpactNotes impact={undo.impact} {modNames} {name} />
      {#if undo.also_undoes.length > 0}
        <p>Also undoes, because it was made later on the same patch:</p>
        <ul class="list" aria-label="Also undone">
          {#each undo.also_undoes as id (id)}<li>{fixLabel(id)}</li>{/each}
        </ul>
      {/if}
      {#if undo.asks.length > 0}
        <p>You changed this since; undo leaves it as it is:</p>
        <ul class="list" aria-label="Left as it is">
          {#each undo.asks as ask (ask)}<li>{ask}</li>{/each}
        </ul>
      {/if}
    {/if}
    <div class="row">
      {#if undo}
        <Button variant="primary" disabled={applying} onclick={confirmUndo}>Undo the fix</Button>
        <Button disabled={applying} onclick={() => (undo = null)}>Back</Button>
      {:else}
        <Button variant="primary" disabled={applying || !impact || impact.impact.breaks_file} onclick={keepOff}>{keepOffLabel(s)}</Button>
        {#if s.kind === "fix"}
          <Button disabled={applying} onclick={askUndo}>Undo the fix...</Button>
        {/if}
      {/if}
    </div>
  {/if}
{/snippet}

{#snippet restoreOffer(label: string)}
  {#if backup && !restored}
    {#if restoreAsked}
      <p>The save goes back to how it was before {phase === "search" ? "the first test" : "the test"}. Progress made since is lost.</p>
      <div class="row">
        <Button variant="danger" disabled={applying} onclick={restore}>Restore save</Button>
        <Button disabled={applying} onclick={() => (restoreAsked = false)}>Keep the save</Button>
      </div>
    {:else}
      <div class="row">
        <Button disabled={applying || game.isBusy} onclick={() => (restoreAsked = true)}>{label}</Button>
      </div>
    {/if}
  {/if}
{/snippet}

<!-- A single test's result lives only here, so the dialog stays open while the test runs. -->
<Dialog bind:open title="Find the cause" size="lg" dismissible={!launching && !applying && phase !== "running"}>
  <div class="safe" aria-live="polite">
    <p>{heading}</p>
    {#if searchNote && phase !== "search"}<Callout tone="info">{searchNote}</Callout>{/if}
    {#if cleanupNote && (phase === "ask" || phase === "result")}
      <Callout tone="warn">The test ran, but putting the game folder back afterwards failed: {cleanupNote}</Callout>
    {/if}

    {#if phase === "intro"}
      {#if suspect === undefined}
        <div aria-busy="true" aria-label="Reading the crash report"><Skeleton lines={3} /></div>
      {:else if suspect === null}
        <p>{nothingNamed} Search the recent changes to find the cause.</p>
        {#if previewError}<p class="error" role="alert">{previewError}</p>{/if}
        {#if launchError}<p class="error" role="alert">{launchError}</p>{/if}
      {:else}
        <p>The crash report names {suspect.label}.</p>
        <ol class="steps" aria-label="Test steps">
          {#each testSteps(suspect) as step (step)}<li>{step}</li>{/each}
        </ol>
        <Callout tone="info">{SAVE_NOTE}</Callout>
        <section class="preview" aria-label="What the test runs">
          {#if previewError}
            <p class="error" role="alert">Could not check the test: {previewError}</p>
            <div class="row"><Button onclick={retryPreview}>Check again</Button></div>
          {:else if !preview}
            <div aria-busy="true" aria-label="Checking the test"><Skeleton lines={2} /></div>
          {:else}
            <ImpactNotes impact={preview.impact} {modNames} {name} />
          {/if}
        </section>
        {#if launchError}<p class="error" role="alert">{launchError}</p>{/if}
        {#if game.isBusy}<p class="qm-muted">Close the game to start the test.</p>{/if}
      {/if}
    {:else if phase === "running"}
      <p>Test running without {suspect?.label}. Play until the problem would show, then quit the game.</p>
      <p class="qm-muted">This window stays open and shows what the test found when the game closes.</p>
    {:else if phase === "ask"}
      <p>QuarterMaster could not tell how the game closed.</p>
      <p id="safe-ask">Did the problem happen?</p>
      <div class="row" role="group" aria-labelledby="safe-ask">
        <Button onclick={() => answer("crashed")}>Yes</Button>
        <Button onclick={() => answer("fine")}>No</Button>
        <Button variant="ghost" onclick={() => answer("stopped")}>Stop</Button>
      </div>
    {:else if phase === "result" && suspect}
      <section class="result" aria-label="Test result">
        {#if verdict === "fine"}
          <p class="verdict">{capitalize(suspect.label)} causes the crash: the game ran without problems without it.</p>
          {@render fixPath(suspect, preview)}
        {:else if verdict === "crashed"}
          <p class="verdict">The problem came back without {suspect.label}, so it is not the cause.</p>
          <p>Search the recent changes next: each test leaves out about half of what is left.</p>
          {#if launchError}<p class="error" role="alert">{launchError}</p>{/if}
        {:else if verdict === "failed"}
          <p class="verdict">The test did not run: {failReason}</p>
        {:else}
          <p class="verdict">Stopped. Nothing in your profile changed.</p>
        {/if}
        {@render restoreOffer("Restore the save from before the test")}
      </section>
    {:else if phase === "search" && search}
      <section class="result" aria-label="Search for the cause">
        {#if search.end === null}
          <p>Searching {search.candidates.length === 1 ? "1 suspect" : `${search.candidates.length} suspects`}: the crash's suspect, fixes changed since the last run without problems, and patches of the file that crashed.</p>
          {#if search.trials.length === 0 && !search.running}
            <ol class="steps" aria-label="Search steps">
              {#each SEARCH_STEPS as step (step)}<li>{step}</li>{/each}
            </ol>
            <Callout tone="info">{SAVE_NOTE}</Callout>
          {/if}
          <div>
            <p id="safe-left">{shownSuspects.length === 1 ? "1 suspect left:" : `${shownSuspects.length} suspects left:`}</p>
            <ul class="suspects" aria-labelledby="safe-left">
              {#each shownSuspects.slice(0, SHOWN_SUSPECTS) as label, i (i)}<li>{label}</li>{/each}
              {#if shownSuspects.length > SHOWN_SUSPECTS}<li class="qm-muted">and {shownSuspects.length - SHOWN_SUSPECTS} more</li>{/if}
            </ul>
          </div>
          {#if search.running}
            <p>{capitalize(testCount(search))} is running without {search.next?.label}. Play until the problem would show, then quit the game.</p>
          {:else if search.ask}
            <p>QuarterMaster could not tell how the game closed.</p>
            <p id="search-ask">Did the problem happen?</p>
            <div class="row" role="group" aria-labelledby="search-ask">
              <Button disabled={searchBusy} onclick={() => answerSearch("crashed")}>Yes</Button>
              <Button disabled={searchBusy} onclick={() => answerSearch("fine")}>No</Button>
              <Button disabled={searchBusy} onclick={() => answerSearch("unknown")}>Not sure</Button>
              <Button variant="ghost" disabled={searchBusy} onclick={stopSearch}>Stop</Button>
            </div>
          {:else}
            {#if search.next}<p>{capitalize(testCount(search))} leaves out {search.next.label}.</p>{/if}
            {#if search.skipped > 0}
              <p class="qm-muted">{search.skipped === 1 ? "1 test was skipped" : `${search.skipped} tests were skipped`} because it would stop a file from compiling.</p>
            {/if}
            <section class="preview" aria-label="What the test runs">
              {#if searchError}
                <p class="error" role="alert">{searchError}</p>
                <div class="row"><Button disabled={searchBusy} onclick={checkNext}>Check again</Button></div>
              {:else if !searchPreview}
                <div aria-busy="true" aria-label="Checking the test"><Skeleton lines={2} /></div>
              {:else}
                <ImpactNotes impact={searchPreview.impact} {modNames} {name} />
              {/if}
            </section>
            {#if game.isBusy}<p class="qm-muted">Close the game to start the test.</p>{/if}
          {/if}
        {:else if found}
          <p class="verdict">{capitalize(found.suspect.label)} causes the crash: the game ran without problems without it.</p>
          {#if searchError}
            <p class="error" role="alert">{searchError}</p>
            {#if !culpritPreview}<div class="row"><Button onclick={previewCulprit}>Check again</Button></div>{/if}
          {/if}
          {@render fixPath(found.suspect, culpritPreview)}
          {@render restoreOffer("Restore the save from before the tests")}
        {:else}
          <p class="verdict">{searchEndText(search)}</p>
          {@render restoreOffer("Restore the save from before the tests")}
        {/if}
      </section>
    {/if}
  </div>
  {#snippet footer()}
    {#if phase === "intro" && suspect}
      <Button onclick={() => (open = false)}>Cancel</Button>
      <Button disabled={!canSearch} onclick={startSearch}>Search recent changes</Button>
      <Button variant="primary" disabled={!canLaunch} aria-busy={launching} onclick={launch}>Test without {suspect.label}</Button>
    {:else if phase === "intro" && suspect === null}
      <Button onclick={() => (open = false)}>Cancel</Button>
      <Button variant="primary" disabled={!canSearch} aria-busy={searchBusy} onclick={startSearch}>Search recent changes</Button>
    {:else if phase === "result" && verdict === "crashed"}
      <Button onclick={() => (open = false)}>Close</Button>
      <Button variant="primary" disabled={!canSearch} aria-busy={searchBusy} onclick={startSearch}>Search recent changes</Button>
    {:else if phase === "search" && search && search.end === null && !search.running && !search.ask}
      <Button disabled={searchBusy || launching} onclick={stopSearch}>Stop</Button>
      <Button variant="primary" disabled={!canLaunchSearch} aria-busy={launching} onclick={launchSearchTest}>Launch {testCount(search)}</Button>
    {:else if phase === "running"}
      <Button disabled>Close</Button>
    {:else}
      <Button onclick={() => (open = false)}>Close</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .safe {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  p,
  ol,
  ul {
    margin: 0;
  }

  .steps {
    padding-left: var(--qm-space-5);
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  .suspects,
  .list {
    padding-left: var(--qm-space-5);
  }

  .preview,
  .result {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .verdict {
    font-weight: 600;
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-2);
  }

  .error {
    color: var(--qm-danger);
  }
</style>
