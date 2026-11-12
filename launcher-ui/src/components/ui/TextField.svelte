<script lang="ts">
  import type { HTMLInputAttributes } from "svelte/elements";

  interface Props extends Omit<HTMLInputAttributes, "value" | "type"> {
    value?: string;
    label: string;
    placeholder?: string;
    type?: "text" | "number" | "password";
    /** Keep the label for screen readers only (a FieldRow already shows it). */
    hideLabel?: boolean;
  }

  let {
    value = $bindable(""),
    label,
    placeholder,
    type = "text",
    hideLabel = false,
    id: idProp,
    oninput,
    class: className = "",
    ...rest
  }: Props = $props();

  const generatedId = $props.id();
  const id = $derived(idProp ?? generatedId);

  function handleInput(e: Event & { currentTarget: EventTarget & HTMLInputElement }) {
    value = e.currentTarget.value;
    oninput?.(e);
  }
</script>

<div class={["field", className]}>
  <label for={id} class={{ "qm-visually-hidden": hideLabel }}>{label}</label>
  <input {...rest} {id} {type} {placeholder} {value} oninput={handleInput} />
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  label {
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }

  input {
    min-height: var(--qm-row-h);
    padding: var(--qm-space-1) var(--qm-space-3);
    border: 1px solid var(--qm-border-strong);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
    color: var(--qm-text);
    font: inherit;
    font-size: var(--qm-text-13);
    transition: border-color var(--qm-dur-fast) var(--qm-ease);
  }

  input::placeholder {
    color: var(--qm-text-muted);
  }

  input:hover:not(:disabled) {
    border-color: var(--qm-accent);
  }

  input:disabled {
    opacity: 0.5;
  }
</style>
