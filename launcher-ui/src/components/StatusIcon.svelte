<script lang="ts">
  import { IconCheck, IconDanger, IconInfo, IconWarn } from "./icons";

  export type StatusTone = "ok" | "warn" | "danger" | "info";

  interface Props {
    tone: StatusTone;
    class?: string;
  }

  let { tone, class: className = "" }: Props = $props();

  // A shape per status, so status never depends on colour alone (the text stays next to it).
  const ICON = { ok: IconCheck, warn: IconWarn, danger: IconDanger, info: IconInfo };
  const Icon = $derived(ICON[tone]);
</script>

<Icon class={["qm-status-icon", className]} data-status={tone} />
