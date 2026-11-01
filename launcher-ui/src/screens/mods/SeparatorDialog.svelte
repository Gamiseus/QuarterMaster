<script lang="ts">
  import { untrack } from "svelte";
  import { Button, Dialog, TextField } from "../../components/ui";
  import { SEP_NAMES, sepColorIndex } from "../../lib/modlist/view";

  interface Props {
    open?: boolean;
    mode: "add" | "edit";
    /** Starting values; `color` may be a legacy hex value. */
    initial: { title: string; color: string };
    onsubmit: (title: string, color: string) => void;
  }

  let { open = $bindable(false), mode, initial, onsubmit }: Props = $props();

  let title = $state("");
  let colorIndex = $state(1);
  const radios: HTMLButtonElement[] = [];

  // Reset the fields each time the dialog opens.
  $effect(() => {
    if (!open) return;
    untrack(() => {
      title = initial.title;
      colorIndex = sepColorIndex(initial.color);
    });
  });

  const valid = $derived(title.trim().length > 0);

  function submit(e?: SubmitEvent) {
    e?.preventDefault();
    if (!valid) return;
    // Keep a legacy value when the color was not changed; any recolor writes "sep-N".
    const unchanged = mode === "edit" && colorIndex === sepColorIndex(initial.color);
    onsubmit(title.trim(), unchanged ? initial.color : `sep-${colorIndex}`);
    open = false;
  }

  function onradiokey(e: KeyboardEvent, i: number) {
    const delta = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (delta === 0) return;
    e.preventDefault();
    const next = ((i - 1 + delta + SEP_NAMES.length) % SEP_NAMES.length) + 1;
    colorIndex = next;
    radios[next - 1]?.focus();
  }
</script>

<Dialog bind:open title={mode === "add" ? "Add separator" : "Edit separator"}>
  <form id="separator-form" class="form" onsubmit={submit}>
    <TextField label="Name" placeholder="Section name" bind:value={title} />
    <div class="colors">
      <span class="label" id="separator-color-label">Color</span>
      <div class="swatches" role="radiogroup" aria-labelledby="separator-color-label">
        {#each SEP_NAMES as name, i (name)}
          <button
            bind:this={radios[i]}
            type="button"
            role="radio"
            class={["swatch", `sep-${i + 1}`]}
            aria-checked={colorIndex === i + 1}
            aria-label={name}
            tabindex={colorIndex === i + 1 ? 0 : -1}
            onclick={() => (colorIndex = i + 1)}
            onkeydown={(e) => onradiokey(e, i + 1)}
          ></button>
        {/each}
      </div>
    </div>
  </form>
  {#snippet footer()}
    <Button variant="ghost" onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" type="submit" form="separator-form" disabled={!valid}>
      {mode === "add" ? "Add" : "Save"}
    </Button>
  {/snippet}
</Dialog>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-4);
  }

  .colors {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  .label {
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }

  .swatches {
    display: flex;
    gap: var(--qm-space-2);
  }

  .swatch {
    --sep: var(--qm-sep-1);
    width: var(--qm-row-h);
    height: var(--qm-row-h);
    padding: 0;
    border: 2px solid transparent;
    border-radius: var(--qm-radius-sm);
    background: var(--sep);
  }

  .swatch[aria-checked="true"] {
    border-color: var(--qm-text);
  }

  .sep-2 {
    --sep: var(--qm-sep-2);
  }

  .sep-3 {
    --sep: var(--qm-sep-3);
  }

  .sep-4 {
    --sep: var(--qm-sep-4);
  }

  .sep-5 {
    --sep: var(--qm-sep-5);
  }

  .sep-6 {
    --sep: var(--qm-sep-6);
  }
</style>
