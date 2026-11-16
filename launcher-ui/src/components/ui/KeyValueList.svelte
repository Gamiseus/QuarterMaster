<script lang="ts" module>
  export interface KeyValueItem {
    label: string;
    value: string;
    /** Paths, hashes and ids: monospace, wrapping anywhere. */
    mono?: boolean;
    /** Clicking the value copies it. */
    copy?: boolean;
  }
</script>

<script lang="ts">
  import { toasts } from "../../lib/stores/toasts.svelte";

  interface Props {
    items: KeyValueItem[];
    /** Accessible name of the list. */
    label: string;
    class?: string;
  }

  let { items, label, class: className = "" }: Props = $props();

  async function copy(item: KeyValueItem) {
    try {
      await navigator.clipboard.writeText(item.value);
      toasts.info("Copied.");
    } catch {
      toasts.error("Could not copy.", "Select the text and copy it manually.");
    }
  }
</script>

<!-- Label and value pairs: a muted label column and a value column, one hairline per row. -->
<ul class={["qm-kv", className]} aria-label={label}>
  {#each items as item (item.label)}
    <li class="row">
      <span class="key">{item.label}</span>
      {#if item.copy}
        <button
          type="button"
          class={["value", "copy", { mono: item.mono }]}
          aria-label="Copy {item.label}"
          title="Copy"
          onclick={() => copy(item)}
        >
          {item.value}
        </button>
      {:else}
        <span class={["value", { mono: item.mono }]}>{item.value}</span>
      {/if}
    </li>
  {/each}
</ul>

<style>
  .qm-kv {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--qm-text-13);
  }

  .row {
    display: grid;
    grid-template-columns: var(--qm-kv-key-w) minmax(0, 1fr);
    gap: var(--qm-space-3);
    align-items: baseline;
    padding: var(--qm-space-2) 0;
    border-bottom: 1px solid var(--qm-border);
  }

  .row:last-child {
    border-bottom: 0;
  }

  .key {
    color: var(--qm-text-muted);
  }

  .value {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .mono {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  .copy {
    padding: 0;
    border: 0;
    background: transparent;
    text-align: left;
    text-decoration: underline dotted var(--qm-border-strong);
    text-underline-offset: 3px;
  }

  .copy:hover {
    color: var(--qm-accent-text);
  }
</style>
