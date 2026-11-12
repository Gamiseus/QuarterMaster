<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    label: string;
    help?: string;
    class?: string;
    /** The control; give it its own accessible label. */
    children?: Snippet;
  }

  let { label, help, class: className = "", children }: Props = $props();

  const uid = $props.id();
</script>

<!-- One setting per row: label and help on the left, the control on the right; stacks when narrow. -->
<div
  class={["qm-field-row", className]}
  role="group"
  aria-labelledby="{uid}-label"
  aria-describedby={help ? `${uid}-help` : undefined}
>
  <div class="inner">
    <div class="text">
      <span class="label" id="{uid}-label">{label}</span>
      {#if help}<span class="qm-muted help" id="{uid}-help">{help}</span>{/if}
    </div>
    <div class="control">{@render children?.()}</div>
  </div>
</div>

<style>
  .qm-field-row {
    container-type: inline-size;
    padding: var(--qm-space-3) 0;
    border-bottom: 1px solid var(--qm-border);
  }

  .qm-field-row:last-child {
    border-bottom: 0;
  }

  .inner {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .text {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    min-width: 0;
  }

  .label {
    font-size: var(--qm-text-14);
  }

  .help {
    font-size: var(--qm-text-12);
  }

  .control {
    display: flex;
    min-width: 0;
  }

  @container (min-width: 520px) {
    .inner {
      display: grid;
      grid-template-columns: minmax(0, 1fr) minmax(0, max-content);
      align-items: center;
      gap: var(--qm-space-4);
    }

    .control {
      justify-content: flex-end;
    }
  }
</style>
