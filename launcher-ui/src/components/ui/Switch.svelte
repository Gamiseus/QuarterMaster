<script lang="ts">
  import { Switch } from "bits-ui";

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
  <Switch.Root bind:checked {disabled} {id} aria-labelledby={labelId} class="qm-switch-track" {...rest}>
    <Switch.Thumb class="qm-switch-thumb" />
  </Switch.Root>
  <label for={id} id={labelId} class={["label", { "visually-hidden": hideLabel }]}>{label}</label>
</span>

<style>
  .field {
    display: inline-flex;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .field :global(.qm-switch-track) {
    display: inline-flex;
    align-items: center;
    width: var(--qm-switch-w);
    height: var(--qm-switch-h);
    padding: 2px;
    border: 1px solid var(--qm-border-strong);
    border-radius: var(--qm-radius-lg);
    background: var(--qm-raised);
    transition:
      background var(--qm-dur-fast) var(--qm-ease),
      border-color var(--qm-dur-fast) var(--qm-ease);
  }

  .field :global(.qm-switch-track[data-state="checked"]) {
    border-color: var(--qm-enabled);
    background: var(--qm-enabled);
  }

  .field :global(.qm-switch-track[data-disabled]) {
    opacity: 0.5;
  }

  .field :global(.qm-switch-thumb) {
    display: block;
    width: var(--qm-switch-thumb);
    height: var(--qm-switch-thumb);
    border-radius: var(--qm-radius-lg);
    background: var(--qm-text-muted);
    transition:
      transform var(--qm-dur-fast) var(--qm-ease),
      background var(--qm-dur-fast) var(--qm-ease);
  }

  .field :global(.qm-switch-thumb[data-state="checked"]) {
    /* Travel = track width less track height (the thumb sits in a square of the track height). */
    transform: translateX(calc(var(--qm-switch-w) - var(--qm-switch-h)));
    background: var(--qm-accent-contrast);
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
