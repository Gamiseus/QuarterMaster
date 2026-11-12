<script lang="ts">
  import { Skeleton } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type FileStat, type PatchExecutionRecord } from "../../../lib/bindings";

  interface Props {
    stats: FileStat[];
    modNames: Record<string, string>;
  }

  let { stats, modNames }: Props = $props();

  // Read-only: every changed file's patches in application order. Reordering is per file.
  let byFile = $state<Record<string, PatchExecutionRecord[] | null>>({});

  $effect(() => {
    for (const s of stats) {
      call(commands.fileDiffGet(s.target_file), { expect: ["not_found"] })
        .then((r) => (byFile = { ...byFile, [s.target_file]: r.status === "ready" ? r.data.result.records : [] }))
        .catch(() => (byFile = { ...byFile, [s.target_file]: [] }));
    }
  });
</script>

<div class="all-order">
  {#each stats as s (s.target_file)}
    {@const recs = byFile[s.target_file]}
    <section aria-label={`Order in ${s.target_file}`}>
      <h4 class="file">{s.target_file}</h4>
      {#if recs === undefined || recs === null}
        <Skeleton lines={2} />
      {:else}
        <ol>
          {#each recs as r (r.patch_id)}
            <li><span class="name">{r.patch_name || r.patch_id}</span> <span class="qm-muted">{modNames[r.mod_id] ?? r.mod_id}</span></li>
          {/each}
        </ol>
      {/if}
    </section>
  {/each}
</div>

<style>
  .all-order {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .file {
    margin: 0 0 var(--qm-space-1);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    font-weight: 600;
  }

  ol {
    margin: 0;
    padding-left: var(--qm-space-5);
  }

  .name {
    font-weight: 600;
  }
</style>
