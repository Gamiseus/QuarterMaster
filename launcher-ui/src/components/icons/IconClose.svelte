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
  stroke-width="1.75"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
  {...rest}
>
  <path d="M4 4l8 8M12 4l-8 8" />
</svg>
