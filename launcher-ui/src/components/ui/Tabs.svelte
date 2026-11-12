<script lang="ts">
  import { Tabs } from "bits-ui";
  import type { Snippet } from "svelte";

  interface Props {
    value?: string;
    tabs: { value: string; label: string }[];
    /** Renders the panel body for one tab value. */
    content: Snippet<[string]>;
    class?: string;
    /** Fill the parent's height: the active panel takes the space under the tab list. */
    fill?: boolean;
  }

  let { value = $bindable(), tabs, content, class: className = "", fill = false }: Props = $props();
</script>

<Tabs.Root bind:value class={["qm-tabs", { "qm-tabs-fill": fill }, className]}>
  <Tabs.List class="qm-tab-list">
    {#each tabs as tab (tab.value)}
      <Tabs.Trigger class="qm-tab" value={tab.value}>{tab.label}</Tabs.Trigger>
    {/each}
  </Tabs.List>
  {#each tabs as tab (tab.value)}
    <Tabs.Content class="qm-tab-panel" value={tab.value}>
      {#if value === tab.value}
        {@render content(tab.value)}
      {/if}
    </Tabs.Content>
  {/each}
</Tabs.Root>

<style>
  :global(.qm-tabs) {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  :global(.qm-tabs-fill) {
    flex: 1;
    min-height: 0;
  }

  /* Only the active panel: bits-ui hides the others with the hidden attribute, which display would override. */
  :global(.qm-tabs-fill > .qm-tab-panel[data-state="active"]) {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }

  :global(.qm-tab-list) {
    flex: none;
    display: flex;
    gap: var(--qm-space-1);
    border-bottom: 1px solid var(--qm-border);
  }

  :global(.qm-tab) {
    margin-bottom: -1px;
    padding: var(--qm-space-2) var(--qm-space-4);
    border: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--qm-text-muted);
    font-size: var(--qm-text-13);
    transition:
      color var(--qm-dur-fast) var(--qm-ease),
      border-color var(--qm-dur-fast) var(--qm-ease);
  }

  :global(.qm-tab:hover) {
    color: var(--qm-text);
  }

  :global(.qm-tab[data-state="active"]) {
    border-bottom-color: var(--qm-accent);
    color: var(--qm-text);
  }
</style>
