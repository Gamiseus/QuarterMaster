<script lang="ts">
  interface Props {
    /** Fraction done, 0..1 (clamped). */
    value: number;
    label: string;
    class?: string;
  }

  let { value, label, class: className = "" }: Props = $props();

  const percent = $derived(Math.round(Math.min(1, Math.max(0, Number.isFinite(value) ? value : 0)) * 100));
</script>

<!-- A native progress element draws the fill, so no inline width style is needed. -->
<progress
  class={["bar", className]}
  max="100"
  value={percent}
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={percent}
  aria-label={label}
></progress>

<style>
  .bar {
    display: block;
    width: 100%;
    height: var(--qm-space-2);
    border: 1px solid var(--qm-border-strong);
    border-radius: var(--qm-radius-sm);
    overflow: hidden;
    background: var(--qm-canvas);
    appearance: none;
  }

  .bar::-webkit-progress-bar {
    background: var(--qm-canvas);
  }

  .bar::-webkit-progress-value {
    background: var(--qm-accent);
    transition: width var(--qm-dur-fast) var(--qm-ease);
  }

  .bar::-moz-progress-bar {
    background: var(--qm-accent);
  }
</style>
