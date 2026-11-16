<script lang="ts">
  import { EmptyState } from "../../components/ui";
  import type { SaveSummary } from "../../lib/bindings";
  import { soldierRows } from "../../lib/saves/dossier";
  import SpriteImage from "./SpriteImage.svelte";

  interface Props {
    summary: SaveSummary;
  }

  let { summary }: Props = $props();

  const rows = $derived(soldierRows(summary));
  /** Award images shown per soldier; the count covers the rest. */
  const SHOWN_AWARDS = 6;
</script>

{#if rows.length === 0}
  <EmptyState title="No soldiers yet." body="Deployed and reserve soldiers show up here once the save has them." />
{:else}
  <table class="soldiers">
    <thead>
      <tr>
        <th scope="col">Name</th>
        <th scope="col">Group</th>
        <th scope="col">Rank</th>
        <th scope="col">Role</th>
        <th scope="col" class="num">Level</th>
        <th scope="col" class="num">Kills</th>
        <th scope="col">Awards</th>
      </tr>
    </thead>
    <tbody>
      {#each rows as s, i (i)}
        <tr>
          <td>{s.name}</td>
          <td>{s.group}</td>
          <td>{s.rank}</td>
          <td>{s.role}</td>
          <td class="num">{s.level}</td>
          <td class="num">{s.kills}</td>
          <td>
            <span class="awards">
              {#each s.awards.slice(0, SHOWN_AWARDS) as icon, j (j)}
                <SpriteImage kind="award" id={icon} alt="Award {icon}" class="award" />
              {/each}
              <span class="qm-muted count">{s.awards.length}</span>
            </span>
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

<style>
  .soldiers {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--qm-text-13);
  }

  th {
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
    font-weight: 400;
    text-align: left;
  }

  th,
  td {
    padding: var(--qm-space-1) var(--qm-space-2);
    border-bottom: 1px solid var(--qm-border);
  }

  .num {
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .awards {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }

  .awards :global(.award) {
    width: 16px;
    height: 16px;
  }

  .count {
    margin-left: var(--qm-space-1);
    font-size: var(--qm-text-12);
  }
</style>
