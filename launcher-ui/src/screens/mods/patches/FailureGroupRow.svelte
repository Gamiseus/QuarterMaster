<script lang="ts">
  import { Button } from "../../../components/ui";
  import { groupText, type ConflictRow } from "../../../lib/patches/groups";

  interface Props {
    row: Extract<ConflictRow, { kind: "group" }>;
    modName: (id: string) => string;
    /** Opens the resolver for one conflict. */
    onresolve: (conflictId: string) => void;
  }

  let { row, modName, onresolve }: Props = $props();

  const n = $derived(row.members.length);
</script>

<div class="failure-group" data-testid="failure-group">
  <p class="title">{groupText(row.group, modName)}</p>
  {#each row.root as c (c.id)}
    <div class="line">
      <p>{c.description}</p>
      <Button size="sm" onclick={() => onresolve(c.id)}>Resolve...</Button>
    </div>
  {/each}
  <details>
    <summary>{n === 1 ? "Show the patch" : `Show the ${n} patches`}</summary>
    <ul aria-label="Patches that fail with it">
      {#each row.members as m (m.member.patch_id)}
        <li>
          <span class="name">{m.member.name}</span>
          <span class="qm-muted">{modName(m.member.mod_id)}</span>
          {#each m.conflicts as c (c.id)}
            <div class="line">
              <p>{c.description}</p>
              <Button size="sm" variant="ghost" onclick={() => onresolve(c.id)}>Resolve...</Button>
            </div>
          {/each}
        </li>
      {/each}
    </ul>
  </details>
</div>

<style>
  .failure-group {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  p {
    margin: 0;
  }

  .title {
    font-weight: 600;
  }

  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-3);
  }

  summary {
    cursor: pointer;
    color: var(--qm-text-muted);
  }

  ul {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: var(--qm-space-1) 0 0;
    padding: 0 0 0 var(--qm-space-4);
  }

  .name {
    font-weight: 600;
  }
</style>
