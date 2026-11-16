<script lang="ts">
  import { untrack } from "svelte";
  import { Button, Dialog, RadioGroup, Skeleton } from "../../components/ui";
  import type { DetectedPaths, LaunchMethod } from "../../lib/bindings";
  import { nextStep, pathsPatch, prevStep, type OnboardingStep, type PathKey } from "../../lib/settings/form";
  import { app } from "../../lib/stores/app.svelte";
  import { setup } from "../../lib/stores/setup.svelte";
  import PathField from "../settings/PathField.svelte";

  const STEP_TITLES: Record<OnboardingStep, string> = {
    game: "The game and your saves",
    workspace: "Workspace folder",
    launch: "Launch method",
  };
  const STEP_NUMBER: Record<OnboardingStep, number> = { game: 1, workspace: 2, launch: 3 };
  const GAME_FIELDS: { key: Exclude<PathKey, "workspace_mods_dir">; label: string; kind: "exe" | "dir" }[] = [
    { key: "game_exe_path", label: "Game executable (mnb.exe)", kind: "exe" },
    { key: "save_dir_path", label: "Save folder", kind: "dir" },
  ];
  const METHODS: { value: LaunchMethod; label: string; description: string }[] = [
    { value: "steam_url", label: "Steam", description: "Starts the game through Steam. Best for a Steam copy." },
    { value: "direct_exe", label: "Direct", description: "Starts mnb.exe directly." },
  ];

  let step = $state<OnboardingStep>("game");
  let found = $state<DetectedPaths | null>(null);
  let draft = $state<Record<PathKey, string | null>>({ game_exe_path: null, save_dir_path: null, workspace_mods_dir: null });
  /** The folder QuarterMaster suggests for the workspace, beside the program. */
  let suggested = $state<string | null>(null);
  let method = $state<LaunchMethod>("direct_exe");
  let busy = $state(false);
  /** Closed here while the "done" answer is on its way, so the dialog never flickers back. */
  let dismissed = $state(false);
  let started = false;

  // Earlier data in AppData is offered first: taking it over makes this setup unnecessary.
  const shown = $derived(
    app.startupDone && app.settings?.first_run_completed === false && !dismissed && setup.legacyChecked && setup.legacy === null,
  );

  async function detect() {
    const s = app.settings;
    draft = { game_exe_path: s?.game_exe_path ?? null, save_dir_path: s?.save_dir_path ?? null, workspace_mods_dir: s?.workspace_mods_dir ?? null };
    try {
      const d = await app.autoDetect();
      found = d;
      for (const f of GAME_FIELDS) draft[f.key] = d[f.key] ?? draft[f.key];
      suggested = d.workspace_mods_dir;
      draft.workspace_mods_dir = draft.workspace_mods_dir ?? d.workspace_mods_dir;
      method = d.is_steam ? "steam_url" : "direct_exe";
    } catch {
      // call() reported it; the user can still pick the folders by hand.
      found = { game_exe_path: null, save_dir_path: null, workspace_mods_dir: null, library_dir_path: "", is_steam: false, game_version: null };
    }
  }

  /** One field's Auto-detect: looks again and returns what was found for it. */
  async function detectOne(key: PathKey): Promise<string | null> {
    const d = await app.autoDetect();
    if (key === "game_exe_path") method = d.is_steam ? "steam_url" : "direct_exe";
    return d[key];
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
</script>

<Dialog
  bind:open={() => shown, (v) => !v && later()}
  title="Welcome to QuarterMaster"
  description="Three quick steps to get the game ready for mods. You can change all of this later in Settings."
  size="lg"
  explicitClose
>
  <div class="steps">
    <p class="qm-muted count">Step {STEP_NUMBER[step]} of 3</p>
    <h3 class="step-title">{STEP_TITLES[step]}</h3>
    {#if step === "game"}
      {#if !found}
        <div aria-busy="true" aria-label="Looking for the game"><Skeleton lines={5} /></div>
      {:else}
        <p class="qm-muted intro">
          {#if found.game_exe_path}
            QuarterMaster looked for the game and found it{found.game_version ? ` (version ${found.game_version})` : ""}.
            Check the two paths; Auto-detect looks again, Browse picks another.
          {:else}
            QuarterMaster could not find the game by itself. Press Auto-detect to look again, or Browse to
            <code>mnb.exe</code> in the Mud and Blood folder.
          {/if}
        </p>
        {#each GAME_FIELDS as f (f.key)}
          <PathField
            label={f.label}
            kind={f.kind}
            value={draft[f.key]}
            highlightDetect
            detect={() => detectOne(f.key)}
            onchange={(p) => (draft[f.key] = p)}
          />
        {/each}
      {/if}
    {:else if step === "workspace"}
      <p class="intro">
        The workspace is a folder for <strong>making</strong> mods: each folder inside it is a mod you are working on,
        and QuarterMaster can run them as they are. Mods you install do not go here; they go into QuarterMaster's
        library.
      </p>
      <PathField label="Workspace mods folder" kind="dir" value={draft.workspace_mods_dir} onchange={(p) => (draft.workspace_mods_dir = p)}>
        {#snippet actions()}
          <Button
            size="sm"
            variant="ghost"
            disabled={suggested === null || draft.workspace_mods_dir === suggested}
            onclick={() => (draft.workspace_mods_dir = suggested)}
          >
            Use the suggested folder
          </Button>
        {/snippet}
      </PathField>
      <ul class="qm-muted notes">
        <li>The suggested folder sits beside QuarterMaster and is made for you. If you only play, leave it as it is.</li>
        <li>Already have a folder of your own mods? Pick it with Browse. Any folder on any drive works.</li>
      </ul>
    {:else}
      <RadioGroup label="Default launch method" items={METHODS} bind:value={method} />
    {/if}
  </div>
  {#snippet footer()}
    <Button variant="ghost" class="later" disabled={busy} onclick={later}>Set up later</Button>
    {#if step !== "game"}
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

  .intro {
    margin: 0;
    font-size: var(--qm-text-13);
  }

  .notes {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    margin: 0;
    padding-left: var(--qm-space-4);
    font-size: var(--qm-text-12);
  }

  code {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  :global(.qm-dialog .later) {
    margin-right: auto;
  }
</style>
