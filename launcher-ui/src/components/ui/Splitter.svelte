<script lang="ts">
  interface Props {
    /** The size it controls, in px (bindable). */
    value: number;
    min: number;
    max: number;
    label: string;
    /** Double click: back to the default size. */
    onreset?: () => void;
    class?: string;
  }

  let { value = $bindable(), min, max, label, onreset, class: className = "" }: Props = $props();

  const STEP = 16;

  let drag: { pointerId: number; startX: number; startValue: number } | null = null;

  const clamp = (v: number) => Math.min(max, Math.max(min, Math.round(v)));

  function onkeydown(e: KeyboardEvent) {
    const next =
      e.key === "ArrowLeft" ? value - STEP
      : e.key === "ArrowRight" ? value + STEP
      : e.key === "Home" ? min
      : e.key === "End" ? max
      : null;
    if (next === null) return;
    e.preventDefault();
    value = clamp(next);
  }

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    drag = { pointerId: e.pointerId, startX: e.clientX, startValue: value };
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    e.preventDefault();
  }

  function onpointermove(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    value = clamp(drag.startValue + (e.clientX - drag.startX));
  }

  function end(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
    drag = null;
  }
</script>

<!-- A vertical bar between two columns: drag, arrow keys (16 px), Home and End; double click resets.
     A focusable separator is an interactive ARIA widget (a window splitter); Svelte does not know that. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class={["qm-splitter", className]}
  role="separator"
  aria-orientation="vertical"
  aria-label={label}
  aria-valuenow={value}
  aria-valuemin={min}
  aria-valuemax={max}
  tabindex="0"
  {onkeydown}
  {onpointerdown}
  {onpointermove}
  onpointerup={end}
  onpointercancel={end}
  ondblclick={() => onreset?.()}
></div>

<style>
  .qm-splitter {
    position: relative;
    flex: none;
    width: var(--qm-space-2);
    cursor: col-resize;
    touch-action: none;
  }

  .qm-splitter::after {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: 50%;
    width: 1px;
    background: var(--qm-border);
    transition: background var(--qm-dur-fast) var(--qm-ease);
  }

  .qm-splitter:hover::after,
  .qm-splitter:focus-visible::after {
    width: 2px;
    background: var(--qm-accent);
  }
</style>
