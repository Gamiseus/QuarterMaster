<script lang="ts">
  import { Button, Dialog, KeyValueList } from "../../components/ui";
  import { setup } from "../../lib/stores/setup.svelte";

  let busy = $state(false);
  const found = $derived(setup.legacy);
  const rows = $derived(
    found
      ? [
          { label: "Found in", value: found.path, mono: true },
          { label: "Mods", value: String(found.mods) },
          { label: "Profiles", value: String(found.profiles) },
        ]
      : [],
  );

  async function move() {
    busy = true;
    try {
      await setup.moveLegacy();
    } catch {
      // call() reported it; the dialog stays to try again or start fresh.
    } finally {
      busy = false;
    }
  }
</script>

<Dialog
  bind:open={() => found !== null, (v) => !v && !busy && setup.declineLegacy()}
  title="Bring your earlier setup along?"
  description="An earlier QuarterMaster kept its mods, profiles, saves and settings in your AppData folder. This one keeps them in its own folder."
  dismissible={!busy}
  explicitClose
>
  <KeyValueList label="Earlier QuarterMaster data" items={rows} />
  <p class="qm-muted note">
    Moving takes the whole folder: mods, profiles, save slots, backups and settings. QuarterMaster restarts when it is
    done, and the old folder is removed. Starting fresh leaves the old folder where it is.
  </p>
  {#snippet footer()}
    <Button disabled={busy} onclick={() => setup.declineLegacy()}>Start fresh</Button>
    <Button variant="primary" disabled={busy} onclick={move}>{busy ? "Moving..." : "Move it here"}</Button>
  {/snippet}
</Dialog>

<style>
  .note {
    margin: var(--qm-space-3) 0 0;
    font-size: var(--qm-text-13);
  }
</style>
