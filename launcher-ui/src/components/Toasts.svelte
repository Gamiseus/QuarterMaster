<script lang="ts">
  import { jobs } from "../lib/stores/jobs.svelte";
  import { toasts, type Toast } from "../lib/stores/toasts.svelte";
  import { IconClose } from "./icons";
  import { Badge, Button, IconButton, ProgressBar } from "./ui";

  const BADGE = { info: "Info", error: "Error", progress: "Working" } as const;
  const TONE = { info: "info", error: "danger", progress: "neutral" } as const;

  function jobOf(toast: Toast) {
    return toast.tone === "progress" ? jobs.list.find((j) => j.id === toast.jobId) : undefined;
  }
</script>

<div class="toasts" aria-live="polite">
  {#each toasts.items as toast (toast.id)}
    <div class="toast" class:error={toast.tone === "error"} role={toast.tone === "error" ? "alert" : "status"}>
      <div class="row">
        <Badge tone={TONE[toast.tone]}>{BADGE[toast.tone]}</Badge>
        <p class="message">{toast.message}</p>
        {#each [...(toast.action ? [toast.action] : []), ...(toast.actions ?? [])] as action, i (i)}
          <Button
            size="sm"
            onclick={() => {
              toasts.dismiss(toast.id);
              action.run();
            }}>{action.label}</Button
          >
        {/each}
        <IconButton icon={IconClose} label="Dismiss" onclick={() => toasts.dismiss(toast.id)} />
      </div>
      {#if toast.tone === "progress"}
        {@const job = jobOf(toast)}
        <ProgressBar class="progress" value={job?.progress ?? 0} label={toast.message} />
        {#if job && job.label && job.label !== toast.message}
          <p class="step">{job.label}</p>
        {/if}
      {/if}
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

  .toasts :global(.progress) {
    margin-top: var(--qm-space-2);
  }

  .step {
    margin: var(--qm-space-1) 0 0;
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
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
