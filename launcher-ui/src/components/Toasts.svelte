<script lang="ts">
  import { toasts } from "../lib/stores/toasts.svelte";
</script>

<div class="toasts" aria-live="polite">
  {#each toasts.items as toast (toast.id)}
    <div class="toast" class:error={toast.tone === "error"} role={toast.tone === "error" ? "alert" : "status"}>
      <div class="row">
        <p class="message">{toast.message}</p>
        {#if toast.action}
          {@const action = toast.action}
          <button
            type="button"
            class="qm-btn"
            onclick={() => {
              toasts.dismiss(toast.id);
              action.run();
            }}>{action.label}</button
          >
        {/if}
        <button type="button" class="dismiss" aria-label="Dismiss" onclick={() => toasts.dismiss(toast.id)}>
          <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M1.5 1.5l7 7M8.5 1.5l-7 7" /></svg>
        </button>
      </div>
      {#if toast.details}
        <details>
          <summary>Details</summary>
          <pre>{toast.details}</pre>
        </details>
      {/if}
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: var(--qm-space-4);
    bottom: var(--qm-space-4);
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    width: 360px;
    z-index: 10;
  }

  .toast {
    padding: var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-left: 3px solid var(--qm-accent);
    border-radius: var(--qm-radius-md);
    background: var(--qm-surface-raised);
  }

  .toast.error {
    border-left-color: var(--qm-danger);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .message {
    flex: 1;
    margin: 0;
  }

  .dismiss {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: 0;
    border-radius: var(--qm-radius-sm);
    background: transparent;
  }

  .dismiss svg {
    width: 10px;
    height: 10px;
    stroke: currentColor;
  }

  details {
    margin-top: var(--qm-space-2);
    color: var(--qm-text-muted);
  }

  pre {
    margin: var(--qm-space-2) 0 0;
    max-height: 160px;
    overflow: auto;
    white-space: pre-wrap;
    font-size: 12px;
  }
</style>
