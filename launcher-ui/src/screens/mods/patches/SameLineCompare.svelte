<script lang="ts">
  import type { SameLineDetail } from "../../../lib/bindings";
  import { versionText } from "../../../lib/patches/explain";

  interface Props {
    detail: SameLineDetail;
    /** The two mods' names, A first. */
    modA: string;
    modB: string;
  }

  let { detail, modA, modB }: Props = $props();

  const cells = $derived([
    { label: "In the game", text: detail.vanilla, code: true },
    { label: `${modA}'s version`, ...versionText(detail.a_text) },
    { label: `${modB}'s version`, ...versionText(detail.b_text) },
  ]);
</script>

<!-- The line two patches both change, side by side (D49). Keep A or Keep B is in the fixes below. -->
<dl class="compare" aria-label={`Line ${detail.line}, as the game and each mod have it`}>
  {#each cells as c (c.label)}
    <div class="cell">
      <dt class="qm-muted">{c.label}</dt>
      <dd>{#if c.code}<code>{c.text}</code>{:else}{c.text}{/if}</dd>
    </div>
  {/each}
</dl>

<style>
  .compare {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: var(--qm-space-2);
    margin: 0;
  }

  .cell {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    min-width: 0;
    padding: var(--qm-space-2) var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  code {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    white-space: pre-wrap;
  }
</style>
