<script lang="ts">
  import { IconClose } from "../../../components/icons";
  import { Button, Select } from "../../../components/ui";
  import { studio } from "../../../lib/stores/studio.svelte";

  interface Props {
    label: string;
    /** What the list means, under the label. */
    help: string;
    value: string[];
    onchange: (ids: string[]) => void;
  }

  let { label, help, value, onchange }: Props = $props();

  // Two steps, as the old wizard had them: the mod, then one of its patches.
  let mod = $state<string>("");
  let patch = $state<string>("");
  const mods = $derived([...new Set(studio.ids.map((i) => i.mod_id))].map((m) => ({ value: m, label: m })));
  const patches = $derived(
    studio.ids
      .filter((i) => i.mod_id === mod && !value.includes(i.id))
      .map((i) => ({ value: i.id, label: i.name ? `${i.name} (${i.id})` : i.id })),
  );

  function add() {
    if (!patch || value.includes(patch)) return;
    onchange([...value, patch]);
    patch = "";
  }
</script>

<fieldset class="picker">
  <legend class="head">{label}</legend>
  <p class="qm-muted help">{help}</p>
  {#if value.length > 0}
    <ul class="chips">
      {#each value as id (id)}
        <li class="chip">
          <span>{id}</span>
          <button type="button" aria-label={`Remove ${id} from ${label}`} onclick={() => onchange(value.filter((x) => x !== id))}>
            <IconClose />
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  <div class="row">
    <Select label={`${label}: mod`} hideLabel placeholder="Mod" items={mods} bind:value={() => mod, (v) => ((mod = v ?? ""), (patch = ""))} />
    <Select label={`${label}: patch`} hideLabel placeholder="Patch" items={patches} disabled={!mod} bind:value={() => patch, (v) => (patch = v ?? "")} />
    <Button size="sm" disabled={!patch} onclick={add}>Add</Button>
  </div>
</fieldset>

<style>
  .picker {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }

  .head {
    padding: 0;
    font-size: var(--qm-text-13);
    font-weight: 600;
  }

  .help {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 1px var(--qm-space-1) 1px var(--qm-space-2);
    border: 1px solid var(--qm-border-strong);
    border-radius: var(--qm-radius-sm);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  .chip button {
    display: inline-flex;
    padding: 0;
    border: 0;
    background: none;
    color: var(--qm-text-muted);
    cursor: pointer;
  }

  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1.4fr) auto;
    gap: var(--qm-space-2);
    align-items: center;
  }
</style>
