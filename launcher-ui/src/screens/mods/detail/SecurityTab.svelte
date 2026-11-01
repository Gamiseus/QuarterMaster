<script lang="ts">
  import { Badge, StencilHeading } from "../../../components/ui";
  import type { IntegrityVerifyResult, SecurityCapability, SecurityScanReport } from "../../../lib/bindings";

  interface Props {
    report: SecurityScanReport;
    integrity: IntegrityVerifyResult;
  }

  let { report, integrity }: Props = $props();

  const CAPABILITY: Record<SecurityCapability, string> = {
    process_execution: "Runs programs or deletes and renames files",
    native_ffi: "Loads native code",
    network_access: "Uses the network",
    raw_filesystem_io: "Reads or writes files outside the game sandbox",
    dynamic_code_execution: "Runs code built at runtime",
  };

  const tone = (severity: string) => (severity === "high" ? "danger" : severity === "medium" ? "warn" : "neutral");
  const sentence = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
  const changed = $derived([
    { label: "Modified", files: integrity.modified },
    { label: "Missing", files: integrity.missing },
    { label: "Untracked", files: integrity.untracked },
  ].filter((g) => g.files.length > 0));
</script>

<div class="security">
  <section>
    <StencilHeading level={3}>Static scan</StencilHeading>
    {#if report.has_high_risk}
      <p><Badge tone="danger">High risk</Badge> This mod uses capabilities that can affect your system.</p>
    {/if}
    {#if report.capabilities.length === 0}
      <p class="qm-muted">No risky capabilities found.</p>
    {:else}
      <ul class="list">
        {#each report.capabilities as cap (cap)}
          <li>{CAPABILITY[cap] ?? cap}</li>
        {/each}
      </ul>
    {/if}
  </section>

  {#if report.findings.length > 0}
    <section>
      <StencilHeading level={3}>Findings</StencilHeading>
      <ul class="findings">
        {#each report.findings as f, i (i)}
          <li>
            <div class="where">
              <Badge tone={tone(f.severity)}>{sentence(f.severity)}</Badge>
              <code>{f.file}:{f.line}</code>
            </div>
            <pre>{f.snippet || f.pattern}</pre>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <section>
    <StencilHeading level={3}>Integrity</StencilHeading>
    {#if integrity.is_pristine}
      <p class="qm-muted">Files match the installed version.</p>
    {:else}
      {#each changed as group (group.label)}
        <p>{group.label}: {group.files.length}</p>
        <ul class="list files">
          {#each group.files as file (file)}
            <li><code>{file}</code></li>
          {/each}
        </ul>
      {/each}
    {/if}
  </section>
</div>

<style>
  .security {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-4);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  p {
    margin: 0;
  }

  .list {
    margin: 0;
    padding-left: var(--qm-space-5);
  }

  .findings {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .where {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
  }

  code,
  pre {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    overflow-wrap: anywhere;
  }

  pre {
    margin: var(--qm-space-1) 0 0;
    padding: var(--qm-space-2);
    overflow-x: auto;
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
    white-space: pre-wrap;
  }
</style>
