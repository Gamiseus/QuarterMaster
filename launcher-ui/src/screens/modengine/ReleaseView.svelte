<script lang="ts">
  import { Badge, Button, Callout, TextField } from "../../components/ui";
  import { call } from "../../lib/api";
  import { commands, type ReleaseInfo, type ReleaseZip } from "../../lib/bindings";
  import { changelogLines, publishSnippet, registryText, releasePageUrl } from "../../lib/engine/release";
  import { openExternal } from "../../lib/external";
  import { formatBytes } from "../../lib/saves/format";
  import { toasts } from "../../lib/stores/toasts.svelte";

  interface Props {
    folder: string;
  }

  let { folder }: Props = $props();

  let info = $state<ReleaseInfo | null>(null);
  let version = $state("");
  let notes = $state("");
  let prepared = $state(false);
  let busy = $state(false);
  let zip = $state<ReleaseZip | null>(null);
  let packError = $state<string | null>(null);

  async function load() {
    info = await call(commands.releaseInfo(folder));
    zip = info.last_zip;
  }

  $effect(() => {
    void folder;
    prepared = false;
    version = "";
    notes = "";
    packError = null;
    load().catch(() => {});
  });

  const current = $derived(info?.json_version ?? info?.toml_version ?? null);
  const mismatch = $derived(info !== null && info.json_version !== null && info.toml_version !== null && info.json_version !== info.toml_version);
  const lines = $derived(changelogLines(notes));
  const releaseVersion = $derived(prepared ? version : (zip?.version || current || ""));
  const releaseLines = $derived(prepared ? lines : (info?.changelog_lines ?? []));

  async function prepare() {
    busy = true;
    try {
      await call(commands.releasePrepare(folder, version.trim(), lines));
      prepared = true;
      await load();
      // A zip of the version before is not this release's.
      if (zip && zip.version !== version.trim()) zip = null;
      toasts.info(`${info?.name ?? folder} is now version ${version.trim()}.`);
    } catch {
      // call() reported it.
    } finally {
      busy = false;
    }
  }

  async function pack() {
    busy = true;
    packError = null;
    try {
      zip = await call(commands.releasePack(folder), { expect: ["invalid_input"] });
    } catch (e) {
      packError = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function copy(text: string, what: string) {
    try {
      await navigator.clipboard.writeText(text);
      toasts.info(`Copied ${what}.`);
    } catch {
      toasts.error("The text could not be copied.");
    }
  }
</script>

<section class="release" aria-label="Release">
  {#if info}
    <ol class="steps">
      <li class="step">
        <h2 class="qm-stencil head">1. Version</h2>
        <p class="qm-muted">
          Now {current ?? "no version"}{#if mismatch}; mod.json says {info.json_version} and qm.toml says {info.toml_version}{/if}.
        </p>
        <div class="row">
          {#each info.next as v, i (v)}
            <Button size="sm" variant={version === v ? "primary" : "secondary"} onclick={() => (version = v)}>
              {["Fix", "Feature", "Major"][i]}: {v}
            </Button>
          {/each}
          <div class="field"><TextField label="New version" hideLabel placeholder="New version" bind:value={version} /></div>
        </div>
      </li>

      <li class="step">
        <h2 class="qm-stencil head">2. Changelog</h2>
        <label class="area">
          <span>What changed, one line each. They go into CHANGELOG.md under the new version.</span>
          <textarea rows="4" bind:value={notes} placeholder="Longer radio range"></textarea>
        </label>
        <div class="row">
          <Button variant="primary" disabled={busy || !version.trim()} onclick={prepare}>Write version and changelog</Button>
          {#if prepared}<Badge tone="ok">Written</Badge>{/if}
        </div>
      </li>

      <li class="step">
        <h2 class="qm-stencil head">3. Check and pack</h2>
        <p class="qm-muted">The full check runs first; the zip is made only when it passes. It goes to _releases\{folder} in the workspace folder.</p>
        <div class="row">
          <Button variant="primary" disabled={busy} aria-busy={busy} onclick={pack}>Check and pack</Button>
          <Button variant="ghost" onclick={() => call(commands.releaseReveal(folder)).catch(() => {})}>Open the folder</Button>
        </div>
        {#if packError}<Callout tone="danger" title="Not packed">{packError}</Callout>{/if}
        {#if zip}
          <dl class="zip">
            <dt>Zip</dt><dd class="mono">{zip.version ? `${zip.name} (${zip.version})` : zip.name}, {formatBytes(zip.size_bytes)}</dd>
            <dt>SHA-256</dt><dd class="mono sha">{zip.sha256}</dd>
          </dl>
        {/if}
      </li>

      <li class="step">
        <h2 class="qm-stencil head">4. Publish</h2>
        {#if info.repository}
          <p class="qm-muted">GitHub's new release page for {info.repository} opens with the tag, title and changelog filled in. Attach the zip there and publish.</p>
          <div class="row">
            <Button
              variant="primary"
              disabled={!zip || !releaseVersion}
              onclick={() => info?.repository && openExternal(releasePageUrl(info.repository, releaseVersion, info.name, releaseLines))}
            >
              Open new release page
            </Button>
          </div>
        {:else}
          <Callout tone="info" title="No repository">Add repository to the [publish] table of qm.toml, or github to mod.json, to open its release page from here.</Callout>
        {/if}
        {#if !info.has_publish}
          <p class="qm-muted">To be listed on the site and in the launcher's Browse tab, add this to qm.toml:</p>
          <pre class="snippet">{publishSnippet(info.repository ?? "", info.asset_name)}</pre>
          <Button size="sm" variant="ghost" onclick={() => copy(publishSnippet(info?.repository ?? "", info?.asset_name ?? ""), "the [publish] table")}>Copy</Button>
          {@const reg = registryText(info.repository ?? "")}
          <p class="qm-muted">Then add the topic quartermaster-mod to the repository, or register it with a pull request that adds {reg.file}:</p>
          <pre class="snippet">{reg.text}</pre>
          <Button size="sm" variant="ghost" onclick={() => copy(reg.text, "the registry line")}>Copy</Button>
        {/if}
        {#if !info.has_readme}<p class="warn">The mod has no README.md; it becomes the mod's page on the site.</p>{/if}
      </li>
    </ol>
  {/if}
</section>

<style>
  .release {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding-right: var(--qm-space-2);
  }

  .steps {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-5);
    max-width: calc(var(--qm-detail-w) * 1.4);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .step {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .head {
    margin: 0;
    font-size: var(--qm-text-14);
  }

  .step p {
    margin: 0;
    font-size: var(--qm-text-13);
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .field {
    width: 10rem;
  }

  .area {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    font-size: var(--qm-text-13);
  }

  textarea {
    padding: var(--qm-space-2);
    border: 1px solid var(--qm-border-strong);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
    color: var(--qm-text);
    font: inherit;
    font-size: var(--qm-text-13);
    resize: vertical;
  }

  .zip {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: var(--qm-space-1) var(--qm-space-3);
    margin: 0;
    font-size: var(--qm-text-13);
  }

  .zip dd {
    margin: 0;
  }

  .mono,
  .snippet {
    font-family: var(--qm-font-mono);
  }

  .sha {
    overflow-wrap: anywhere;
  }

  .snippet {
    margin: 0;
    padding: var(--qm-space-2);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-panel);
    font-size: var(--qm-text-12);
    white-space: pre-wrap;
  }

  .warn {
    color: var(--qm-warn);
  }
</style>
