<script lang="ts">
  import { untrack } from "svelte";
  import { Button, Dialog, RadioGroup, Skeleton } from "../../components/ui";
  import type { DetectedPaths, LaunchMethod } from "../../lib/bindings";
  import { nextStep, pathsPatch, prevStep, type OnboardingStep, type PathKey } from "../../lib/settings/form";
  import { app } from "../../lib/stores/app.svelte";
  import PathField from "../settings/PathField.svelte";

  const STEP_TITLES: Record<OnboardingStep, string> = {
    detect: "Find the game",
    confirm: "Check folders",
    launch: "Launch method",
  };
  const STEP_NUMBER: Record<OnboardingStep, number> = { detect: 1, confirm: 2, launch: 3 };
  const FIELDS: { key: PathKey; label: string; kind: "exe" | "dir" }[] = [
    { key: "game_exe_path", label: "Game executable", kind: "exe" },
    { key: "save_dir_path", label: "Save folder", kind: "dir" },
    { key: "workspace_mods_dir", label: "Workspace mods folder", kind: "dir" },
  ];
  const METHODS: { value: LaunchMethod; label: string; description: string }[] = [
    { value: "steam_url", label: "Steam", description: "Starts the game through Steam. Best for a Steam copy." },
    { value: "direct_exe", label: "Direct", description: "Starts mnb.exe directly." },
  ];

  let step = $state<OnboardingStep>("detect");
  let found = $state<DetectedPaths | null>(null);
  let draft = $state<Record<PathKey, string | null>>({ game_exe_path: null, save_dir_path: null, workspace_mods_dir: null });
  let method = $state<LaunchMethod>("direct_exe");
  let busy = $state(false);
  /** Closed here while the "done" answer is on its way, so the dialog never flickers back. */
  let dismissed = $state(false);
  let started = false;

  const shown = $derived(app.startupDone && app.settings?.first_run_completed === false && !dismissed);

  async function detect() {
    const s = app.settings;
    draft = { game_exe_path: s?.game_exe_path ?? null, save_dir_path: s?.save_dir_path ?? null, workspace_mods_dir: s?.workspace_mods_dir ?? null };
    try {
      const d = await app.autoDetect();
      found = d;
      for (const f of FIELDS) draft[f.key] = d[f.key] ?? draft[f.key];
      method = d.is_steam ? "steam_url" : "direct_exe";
    } catch {
      // call() reported it; the user can still pick the folders by hand.
      found = { game_exe_path: null, save_dir_path: null, workspace_mods_dir: null, library_dir_path: "", is_steam: false, game_version: null };
    }
  }

  $effect(() => {
    if (shown && !started) {
      started = true;
      untrack(() => void detect());
    }
  });

  async function finish() {
    if (!app.settings || busy) return;
    busy = true;
    try {
      await app.completeOnboarding({ ...pathsPatch(draft, app.settings), default_launch_method: method });
    } catch {
      // A rejected path (shown by call()) keeps the dialog open to fix it.
    } finally {
      busy = false;
    }
  }

  /** Set up later, Escape or Close: onboarding is done with nothing changed. */
  function later() {
    dismissed = true;
    app.completeOnboarding({}).catch(() => {});
  }

  const results = $derived(
    found
      ? [
          { label: "Game", value: found.game_exe_path ?? "Not found" },
          { label: "Save folder", value: found.save_dir_path ?? "Not found" },
          { label: "Steam copy", value: found.is_steam ? "Yes" : "No" },
          { label: "Game version", value: found.game_version ?? "Not detected" },
        ]
      : [],
  );
</script>

<Dialog
  bind:open={() => shown, (v) => !v && later()}
  title="Welcome to QuarterMaster"
  description="Three quick steps to get the game ready for mods. You can change all of this later in Settings."
  size="lg"
>
  <div class="steps">
    <p class="qm-muted count">Step {STEP_NUMBER[step]} of 3</p>
    <h3 class="step-title">{STEP_TITLES[step]}</h3>
    {#if step === "detect"}
      {#if !found}
        <div aria-busy="true" aria-label="Looking for the game"><Skeleton lines={5} /></div>
      {:else}
        <ul class="results" aria-label="What QuarterMaster found">
          {#each results as r (r.label)}
            <li><span class="qm-muted">{r.label}</span><code>{r.value}</code></li>
          {/each}
        </ul>
      {/if}
    {:else if step === "confirm"}
      {#each FIELDS as f (f.key)}
        <PathField label={f.label} kind={f.kind} value={draft[f.key]} onchange={(p) => (draft[f.key] = p)} />
      {/each}
    {:else}
      <RadioGroup label="Default launch method" items={METHODS} bind:value={method} />
    {/if}
  </div>
  {#snippet footer()}
    <Button variant="ghost" class="later" disabled={busy} onclick={later}>Set up later</Button>
    {#if step !== "detect"}
      <Button disabled={busy} onclick={() => (step = prevStep(step))}>Back</Button>
    {/if}
    <!-- One button for Next and Finish, so focus stays on it when the last step opens. -->
    <Button
      variant="primary"
      disabled={busy || !found}
      onclick={() => (step === "launch" ? void finish() : (step = nextStep(step)))}
    >
      {step === "launch" ? "Finish" : "Next"}
    </Button>
  {/snippet}
</Dialog>

<style>
  .steps {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .count {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .step-title {
    margin: 0;
    font-size: var(--qm-text-16);
    font-weight: 600;
  }

  .results {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--qm-text-13);
  }

  .results li {
    display: grid;
    grid-template-columns: calc(var(--qm-detail-w) * 0.5) 1fr;
    gap: var(--qm-space-3);
  }

  .results code {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    overflow-wrap: anywhere;
  }

  :global(.qm-dialog .later) {
    margin-right: auto;
  }
</style>
