<script lang="ts">
  import { open as openPicker } from "@tauri-apps/plugin-dialog";
  import type { Snippet } from "svelte";
  import { Button } from "../../components/ui";
  import { toasts } from "../../lib/stores/toasts.svelte";

  interface Props {
    label: string;
    value: string | null;
    /** `exe` picks mnb.exe; `dir` picks a folder. */
    kind: "exe" | "dir";
    disabled?: boolean;
    /** Called with a path the user picked or that was detected. */
    onchange: (path: string) => void;
    /** Finds the path automatically; null when nothing was found. */
    detect?: () => Promise<string | null>;
    /** More buttons after Browse (and Auto-detect). */
    actions?: Snippet;
  }

  let { label, value, kind, disabled = false, onchange, detect, actions }: Props = $props();

  let detecting = $state(false);

  async function browse() {
    let picked: string | null;
    try {
      picked =
        kind === "dir"
          ? await openPicker({ directory: true })
          : await openPicker({ filters: [{ name: "Mud and Blood", extensions: ["exe"] }] });
    } catch (e) {
      toasts.error("Could not open the file picker.", e instanceof Error ? e.message : String(e));
      return;
    }
    if (picked) onchange(picked);
  }

  async function autoDetect() {
    if (!detect) return;
    detecting = true;
    try {
      const found = await detect();
      if (found === null) toasts.info(`Could not find the ${label.toLowerCase()} automatically.`);
      else if (found !== value) onchange(found);
    } catch {
      // call() already reported it.
    } finally {
      detecting = false;
    }
  }
</script>

<div class="field" role="group" aria-label={label}>
  <span class="label">{label}</span>
  <code class={["path", { unset: !value }]}>{value ?? "Not set"}</code>
  <div class="buttons">
    <Button size="sm" {disabled} onclick={browse}>Browse</Button>
    {#if detect}
      <Button size="sm" variant="ghost" disabled={disabled || detecting} onclick={autoDetect}>Auto-detect</Button>
    {/if}
    {@render actions?.()}
  </div>
</div>

<style>
  .field {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--qm-space-1) var(--qm-space-3);
    align-items: center;
  }

  .label {
    grid-column: 1 / -1;
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }

  .path {
    min-width: 0;
    padding: var(--qm-space-1) var(--qm-space-2);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    overflow-wrap: anywhere;
  }

  .path.unset {
    color: var(--qm-text-muted);
  }

  .buttons {
    display: flex;
    gap: var(--qm-space-2);
  }
</style>
