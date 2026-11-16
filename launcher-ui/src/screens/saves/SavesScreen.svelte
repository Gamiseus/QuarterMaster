<script lang="ts">
  import { untrack } from "svelte";
  import { Screen } from "../../components/ui";
  import { saves } from "../../lib/stores/saves.svelte";
  import SlotDetail from "./SlotDetail.svelte";
  import SlotStrip from "./SlotStrip.svelte";

  // The shell loads the slot list at startup; load it here only if that has not happened.
  $effect(() => {
    untrack(() => {
      if (saves.list === null) saves.load().catch(() => {});
    });
  });
</script>

<!-- The slot strip across the top, the selected slot below it filling the rest (spec 7.1). -->
<Screen title="Saves" scroll={false}>
  <SlotStrip />
  <SlotDetail />
</Screen>
