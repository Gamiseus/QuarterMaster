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
  <path d="M5.5 4h.01M10.5 4h.01M5.5 8h.01M10.5 8h.01M5.5 12h.01M10.5 12h.01" />
</svg>
