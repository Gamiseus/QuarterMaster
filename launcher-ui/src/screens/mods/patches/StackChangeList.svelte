<script lang="ts">
  import type { StackChange } from "../../../lib/bindings";
  import { blockLabel, stackHeading } from "../../../lib/patches/stacks";

  interface Props {
    changes: StackChange[];
    modNames: Record<string, string>;
  }

  let { changes, modNames }: Props = $props();

  const modName = (id: string) => modNames[id] ?? id;
</script>

<ul class="changes" aria-label="Shared lines that change">
  {#each changes as c, i (`${c.file}:${c.line}:${c.side}:${i}`)}
    <li class="change">
      <p class="head"><span class="file">{c.file}</span> {stackHeading(c.line, c.anchor_text, c.side, c.after)}</p>
      <div class="cols">
        <div>
          <p class="qm-muted label">Now</p>
          <ol aria-label="Now">
            {#each c.before as b (b.patch_id)}<li>{blockLabel(b, c.before, modName)}</li>{/each}
          </ol>
        </div>
        <div>
          <p class="qm-muted label">After the change</p>
          <ol aria-label="After the change">
            {#each c.after as b (b.patch_id)}<li>{blockLabel(b, c.after, modName)}</li>{/each}
          </ol>
        </div>
      </div>
    </li>
  {/each}
</ul>

<style>
  .changes {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .change {
    padding: var(--qm-space-2) var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-panel);
  }

  p {
    margin: 0;
  }

  .head {
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .file {
    margin-right: var(--qm-space-2);
    color: var(--qm-text-muted);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    font-weight: 400;
  }

  .cols {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--qm-space-3);
    margin-top: var(--qm-space-1);
  }

  .label {
    font-size: var(--qm-text-12);
  }

  ol {
    margin: 0;
    padding-left: var(--qm-space-5);
  }
</style>
