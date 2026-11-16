<script lang="ts">
  import { call } from "../lib/api";
  import { commands, type IssueArea, type IssueReportResult } from "../lib/bindings";
  import { openExternal } from "../lib/external";
  import { app } from "../lib/stores/app.svelte";
  import { setup } from "../lib/stores/setup.svelte";
  import { Button, Checkbox, Dialog, Select, TextField } from "./ui";

  const AREAS: { value: IssueArea; label: string; description: string }[] = [
    { value: "launcher", label: "The launcher", description: "A button, a screen, installing, updating or anything else in this window." },
    { value: "loader", label: "The game with mods", description: "The game crashes, will not start, or mods do not load." },
    { value: "mod", label: "A mod", description: "One mod misbehaves. Name it in the title. Mods are made by their authors, who may be the ones to fix it." },
    { value: "other", label: "Something else", description: "The website, the documentation or a suggestion." },
  ];
  const MAX_TITLE = 150;
  const MAX_TEXT = 10000;

  let area = $state<IssueArea>("launcher");
  let title = $state("");
  let text = $state("");
  let attach = $state(true);
  let busy = $state(false);
  let done = $state<IssueReportResult | null>(null);

  const ready = $derived(title.trim().length > 0 && text.trim().length > 0);
  const fileName = $derived(done?.file?.split(/[\/]/).pop() ?? null);

  // A fresh form each time it opens; what was typed is kept while it stays open.
  $effect(() => {
    if (!setup.reportOpen) {
      done = null;
      busy = false;
    }
  });

  async function submit() {
    if (!ready || busy) return;
    busy = true;
    try {
      const result = await call(commands.issueReportBuild({ area, title, description: text, attach_files: attach }));
      done = result;
      title = "";
      text = "";
      await openExternal(result.url);
      if (result.file) app.reveal("diagnostics").catch(() => {});
    } catch {
      // call() reported it.
    } finally {
      busy = false;
    }
  }
</script>

<Dialog
  bind:open={() => setup.reportOpen, (v) => !busy && (setup.reportOpen = v)}
  title={done ? "Finish the report on GitHub" : "Report a problem"}
  description={done
    ? undefined
    : "Reports go to QuarterMaster's issue list on GitHub, where anyone can read them. You need a free GitHub account. Nothing is sent from here: your browser opens with the text filled in."}
  size="lg"
  dismissible={!busy}
  explicitClose
>
  {#if done}
    <ol class="steps" data-testid="report-steps">
      <li>Your browser opened a new issue with your text filled in. Sign in to GitHub if it asks.</li>
      {#if fileName}
        <li>
          Drag <code>{fileName}</code> from the folder that just opened onto the issue text. It holds your mod list, load
          order, QuarterMaster settings, the last lines of the logs and recent crash reports.
        </li>
      {/if}
      <li>Read it through, then press <strong>Create</strong> on the page.</li>
    </ol>
  {:else}
    <div class="form">
      <div class="about">
        <Select label="What is it about?" items={AREAS} bind:value={area} />
        <p class="qm-muted help">{AREAS.find((a) => a.value === area)?.description}</p>
      </div>
      <TextField label="Title" placeholder="A short summary, such as: Crash when opening the map" maxlength={MAX_TITLE} bind:value={title} />
      <label class="area">
        <span class="label">What happened?</span>
        <textarea
          rows="6"
          maxlength={MAX_TEXT}
          placeholder="What you did, what you expected and what happened instead. If it happens every time, say how to make it happen."
          bind:value={text}
        ></textarea>
      </label>
      <Checkbox label="Make a report file to attach" bind:checked={attach} />
      <p class="qm-muted help">
        The file helps most with crashes and mods that do not load. It holds your mod list, load order, QuarterMaster
        settings, the last lines of the launcher and game logs and recent crash reports. Your Windows user name and
        Steam id are taken out. It stays on your computer until you add it to the issue yourself.
      </p>
    </div>
  {/if}
  {#snippet footer()}
    {#if done}
      <Button onclick={() => void openExternal(done?.url)}>Open the page again</Button>
      {#if fileName}
        <Button onclick={() => app.reveal("diagnostics").catch(() => {})}>Open the folder</Button>
      {/if}
      <Button variant="primary" onclick={() => (setup.reportOpen = false)}>Done</Button>
    {:else}
      <Button disabled={busy} onclick={() => (setup.reportOpen = false)}>Cancel</Button>
      <Button variant="primary" disabled={!ready || busy} onclick={submit}>{busy ? "Preparing..." : "Open the issue page"}</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .about {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  .area {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  .label {
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }

  textarea {
    width: 100%;
    padding: var(--qm-space-2);
    border: 1px solid var(--qm-border-strong);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
    color: var(--qm-text);
    font: inherit;
    font-size: var(--qm-text-13);
    resize: vertical;
  }

  .help {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .steps {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: 0;
    padding-left: var(--qm-space-5);
    font-size: var(--qm-text-13);
  }

  code {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    overflow-wrap: anywhere;
  }
</style>
