<script lang="ts">
  import { call } from "../../../lib/api";
  import { commands } from "../../../lib/bindings";
  import { MOD_COLOURS } from "../../../lib/patches/colors";
  import { app } from "../../../lib/stores/app.svelte";
  import { modColours } from "../../../lib/stores/modcolours.svelte";

  interface Props {
    modId: string;
    modName: string;
  }

  let { modId, modName }: Props = $props();

  /** The colour picked for this mod; null while the launcher picks one. */
  const picked = $derived(app.settings?.mod_colors?.[modId] ?? null);
  const shown = $derived(modColours([modId]).get(modId) ?? 1);
  let busy = $state(false);

  async function set(colour: number | null) {
    if (busy || colour === picked) return;
    busy = true;
    try {
      app.setSettings(await call(commands.modColorSet(modId, colour)));
    } catch {
      // call() showed the error.
    } finally {
      busy = false;
    }
  }
</script>

<div class="colour" role="group" aria-label={`Colour of ${modName} in the patch views`} data-testid="mod-colour">
  <span class="label">Colour in the patch views</span>
  <span class="swatches">
    {#each Array.from({ length: MOD_COLOURS }, (_, i) => i + 1) as n (n)}
      <button
        type="button"
        class={["swatch", `c${n}`, { on: shown === n }]}
        aria-label={`Colour ${n}`}
        aria-pressed={shown === n}
        disabled={busy}
        onclick={() => set(n)}
      ></button>
    {/each}
    <button type="button" class="auto" disabled={busy || picked === null} onclick={() => set(null)}>Automatic</button>
  </span>
  <span class="qm-muted help">{picked === null ? "Picked by the launcher. Choose one to keep it." : "Your choice, used wherever this mod's changes are shown."}</span>
</div>

<style>
  .colour {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  .label {
    font-size: var(--qm-text-12);
    color: var(--qm-text-muted);
  }

  .swatches {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .swatch {
    width: var(--qm-space-5);
    height: var(--qm-space-5);
    padding: 0;
    border: 2px solid transparent;
    border-radius: 50%;
    background: var(--attr);
    box-shadow: 0 0 0 1px var(--qm-border);
  }

  .swatch.on {
    border-color: var(--qm-panel);
    box-shadow: 0 0 0 2px var(--qm-text);
  }

  .swatch:focus-visible,
  .auto:focus-visible {
    outline: none;
    box-shadow: var(--qm-focus-ring);
  }

  .auto {
    padding: 0 var(--qm-space-2);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: transparent;
    font-size: var(--qm-text-12);
  }

  .auto:disabled {
    opacity: 0.5;
  }

  .help {
    font-size: var(--qm-text-12);
  }

  .c1 { --attr: var(--qm-attr-1); }
  .c2 { --attr: var(--qm-attr-2); }
  .c3 { --attr: var(--qm-attr-3); }
  .c4 { --attr: var(--qm-attr-4); }
  .c5 { --attr: var(--qm-attr-5); }
  .c6 { --attr: var(--qm-attr-6); }
  .c7 { --attr: var(--qm-attr-7); }
  .c8 { --attr: var(--qm-attr-8); }
</style>
