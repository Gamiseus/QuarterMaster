<script lang="ts">
  import { KeyValueList } from "../../../components/ui";
  import AssetFiles from "./AssetFiles.svelte";
  import AutoUpdateField from "./AutoUpdateField.svelte";
  import type { ModDetail } from "../../../lib/bindings";
  import { guardLinks, isHttpsUrl } from "../../../lib/external";
  import { renderMarkdown } from "../../../lib/markdown";
  import { factionSide } from "../../../lib/modlist/status";

  interface Props {
    detail: ModDetail;
  }

  let { detail }: Props = $props();

  const assetsOnly = $derived(detail.summary.kind === "assets");
  const faction = $derived(detail.summary.faction);

  const GITHUB_REPO = /^[\w.-]+\/[\w.-]+$/;

  const readme = $derived(detail.readme_md ? renderMarkdown(detail.readme_md) : null);
  const changelog = $derived(detail.changelog_md ? renderMarkdown(detail.changelog_md) : null);
  const authors = $derived(detail.summary.authors.length > 0 ? detail.summary.authors.join(", ") : "Unknown");
  const links = $derived.by(() => {
    const m = detail.manifest;
    const github = m.github && GITHUB_REPO.test(m.github) ? `https://github.com/${m.github}` : m.github;
    return [
      { label: "Homepage", url: m.homepage },
      { label: "GitHub", url: github },
      { label: "Nexus Mods", url: m.nexus },
    ].filter((l): l is { label: string; url: string } => isHttpsUrl(l.url));
  });
</script>

<div class="overview">
  {#if detail.manifest.description}
    <p class="description">{detail.manifest.description}</p>
  {/if}
  <KeyValueList
    label="Mod facts"
    items={[
      { label: "Authors", value: authors },
      ...(detail.summary.category ? [{ label: "Category", value: detail.summary.category }] : []),
      ...(faction
        ? [{ label: "Faction", value: `${faction.name}. Replaces ${factionSide(faction)} in game; enable one faction per side.` }]
        : []),
      ...(assetsOnly
        ? [{ label: "Type", value: "Assets only. The game loads its files from the assets folder; it has no update source." }]
        : [{ label: "Installed versions", value: detail.summary.installed_versions.join(", ") || detail.summary.version }]),
    ]}
  />
  {#if !detail.summary.is_workspace_linked && !assetsOnly}
    <AutoUpdateField modId={detail.summary.id} />
  {/if}
  {#if links.length > 0}
    <ul class="links" {@attach guardLinks}>
      {#each links as link (link.label)}
        <li><a href={link.url} rel="noopener noreferrer" data-external="">{link.label}</a></li>
      {/each}
    </ul>
  {/if}
  {#if detail.asset_files.length > 0 && assetsOnly}
    <AssetFiles files={detail.asset_files} />
  {/if}
  {#if readme}
    <div class="markdown" {@attach guardLinks}>{@html readme}</div>
  {:else if !assetsOnly}
    <p class="qm-muted">This mod has no readme.</p>
  {/if}
  {#if detail.asset_files.length > 0 && !assetsOnly}
    <AssetFiles files={detail.asset_files} />
  {/if}
  {#if changelog}
    <details class="changelog">
      <summary>Changelog</summary>
      <div class="markdown" {@attach guardLinks}>{@html changelog}</div>
    </details>
  {/if}
</div>

<style>
  .overview {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .description {
    margin: 0;
  }

  .links {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  a,
  .markdown :global(a) {
    color: var(--qm-accent-text);
  }

  .markdown {
    overflow-wrap: anywhere;
  }

  .markdown :global(:is(h1, h2, h3, h4)) {
    margin: var(--qm-space-3) 0 var(--qm-space-2);
    font-family: var(--qm-font-sans);
    font-size: var(--qm-text-14);
    letter-spacing: normal;
    text-transform: none;
  }

  .markdown :global(:is(p, ul, ol)) {
    margin: 0 0 var(--qm-space-2);
  }

  .markdown :global(:is(ul, ol)) {
    padding-left: var(--qm-space-5);
  }

  .markdown :global(pre) {
    overflow-x: auto;
    padding: var(--qm-space-2);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
  }

  .markdown :global(code) {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  .changelog summary {
    cursor: pointer;
    color: var(--qm-text-muted);
  }

  .changelog[open] summary {
    margin-bottom: var(--qm-space-2);
  }
</style>
