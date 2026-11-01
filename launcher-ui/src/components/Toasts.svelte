<script lang="ts">
  import { toasts } from "../lib/stores/toasts.svelte";
  import { IconClose } from "./icons";
  import { Badge, Button, IconButton } from "./ui";
</script>

<div class="toasts" aria-live="polite">
  {#each toasts.items as toast (toast.id)}
    <div class="toast" class:error={toast.tone === "error"} role={toast.tone === "error" ? "alert" : "status"}>
      <div class="row">
        <Badge tone={toast.tone === "error" ? "danger" : "accent"}>{toast.tone === "error" ? "Error" : "Info"}</Badge>
        <p class="message">{toast.message}</p>
        {#if toast.action}
          {@const action = toast.action}
          <Button
            size="sm"
            onclick={() => {
              toasts.dismiss(toast.id);
              action.run();
            }}>{action.label}</Button
          >
        {/if}
        <IconButton icon={IconClose} label="Dismiss" onclick={() => toasts.dismiss(toast.id)} />
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
    z-index: var(--qm-z-toast);
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    width: calc(var(--qm-detail-w) + var(--qm-space-2));
  }

  .toast {
    padding: var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-left: 3px solid var(--qm-accent);
    border-radius: var(--qm-radius-md);
    background: var(--qm-popover);
    color: var(--qm-text);
    font-size: var(--qm-text-13);
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

  summary {
    cursor: pointer;
  }

  details {
    margin-top: var(--qm-space-2);
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
  }

  pre {
    margin: var(--qm-space-2) 0 0;
    max-height: calc(var(--qm-row-h) * 5);
    overflow: auto;
    white-space: pre-wrap;
    font-size: var(--qm-text-12);
  }
</style>
