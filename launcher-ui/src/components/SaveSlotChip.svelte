<script lang="ts">
  import { profiles } from "../lib/stores/profiles.svelte";
  import { saves } from "../lib/stores/saves.svelte";
  import { IconSaves } from "./icons";
  import { Badge } from "./ui";

  const slotId = $derived(profiles.active?.linked_save_profile_id ?? null);
</script>

{#if slotId}
  <span class="chip">
    <Badge data-testid="save-slot-chip">
      <span class="inner">
        <IconSaves />
        <span class="name">{saves.nameFor(slotId)}</span>
      </span>
    </Badge>
  </span>
{/if}

<style>
  .chip {
    display: flex;
    min-width: 0;
    max-width: calc(var(--qm-detail-w) / 2);
  }

  /* Badge's root is the only child element of the wrapper. */
  .chip > :global(span) {
    min-width: 0;
    max-width: 100%;
  }

  .inner {
    display: inline-flex;
    align-items: center;
    gap: var(--qm-space-1);
    min-width: 0;
  }

  .inner :global(.qm-icon) {
    flex: none;
    width: var(--qm-text-12);
    height: var(--qm-text-12);
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
