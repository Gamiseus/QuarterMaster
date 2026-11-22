<script lang="ts">
  import type { SVGAttributes } from "svelte/elements";

  let { class: className = "", ...rest }: SVGAttributes<SVGSVGElement> = $props();
</script>

<svg
  class={["qm-icon", className]}
  viewBox="0 0 16 16"
  width="16"
  height="16"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
  {...rest}
>
  <path d="M2 4.5V12a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V6a1 1 0 0 0-1-1H8L6.5 3.5H3a1 1 0 0 0-1 1z" />
  <path d="M8 7v4M6 9h4" />
</svg>
