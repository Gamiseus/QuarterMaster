<script lang="ts">
  import { Checkbox } from "bits-ui";
  import IconCheck from "../icons/IconCheck.svelte";

  interface Props {
    checked?: boolean;
    label: string;
    hideLabel?: boolean;
    disabled?: boolean;
    class?: string;
    id?: string;
    [key: string]: unknown;
  }

  let {
    checked = $bindable(false),
    label,
    hideLabel = false,
    disabled = false,
    id: idProp,
    class: className = "",
    ...rest
  }: Props = $props();

  const uid = $props.id();
  const labelId = `${uid}-label`;
  const id = $derived(idProp ?? `${uid}-control`);
</script>

<span class={["field", className]}>
  <Checkbox.Root bind:checked {disabled} {id} aria-labelledby={labelId} class="qm-checkbox-box" {...rest}>
    {#snippet children({ checked: on })}
      {#if on}
        <IconCheck />
      {/if}
    {/snippet}
  </Checkbox.Root>
  <label for={id} id={labelId} class={["label", { "visually-hidden": hideLabel }]}>{label}</label>
</span>

<style>
  .field {
    display: inline-flex;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .field :global(.qm-checkbox-box) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
    color: var(--qm-accent-contrast);
    transition:
      background var(--qm-dur-fast) var(--qm-ease),
      border-color var(--qm-dur-fast) var(--qm-ease);
  }

  .field :global(.qm-checkbox-box:hover:not([data-disabled])) {
    border-color: var(--qm-accent);
  }

  .field :global(.qm-checkbox-box[data-state="checked"]) {
    border-color: var(--qm-accent);
    background: var(--qm-accent);
  }

  .field :global(.qm-checkbox-box[data-disabled]) {
    opacity: 0.5;
  }

  .label {
    font-size: var(--qm-text-13);
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
