<script lang="ts">
  import { call } from "../../lib/api";
  import { commands, type ProfileDiff } from "../../lib/bindings";
  import { Dialog, EmptyState, Section, Select, Skeleton } from "../../components/ui";
  import { mods } from "../../lib/stores/mods.svelte";
  import { profiles } from "../../lib/stores/profiles.svelte";

  interface Props {
    open?: boolean;
    /** The profile to start from (A). */
    start?: string | null;
  }

  let { open = $bindable(false), start = null }: Props = $props();

  let a = $state<string | undefined>(undefined);
  let b = $state<string | undefined>(undefined);
  let diff = $state<ProfileDiff | null>(null);
  let failed = $state(false);
  let seq = 0;

  const items = $derived(profiles.list.map((p) => ({ value: p.id, label: p.name })));
  const nameOf = (id: string | undefined) => profiles.list.find((p) => p.id === id)?.name ?? id ?? "";
  const modName = (id: string) => (mods.list.status === "ready" ? mods.list.data.find((m) => m.id === id)?.name : undefined) ?? id;

  // Each time it opens: A is the given profile, B the first other one.
  $effect(() => {
    if (!open) return;
    a = start ?? profiles.activeSummary?.id ?? profiles.list[0]?.id;
    b = profiles.list.find((p) => p.id !== a)?.id;
  });

  $effect(() => {
    const [x, y] = [a, b];
    diff = null;
    failed = false;
    if (!open || !x || !y || x === y) return;
    const mine = ++seq;
    call(commands.profileDiff(x, y))
      .then((d) => {
        if (mine === seq) diff = d;
      })
      .catch(() => {
        if (mine === seq) failed = true;
      });
  });
</script>

{#snippet none()}<p class="qm-muted none">None.</p>{/snippet}

<Dialog bind:open title="Compare profiles" size="lg">
  <div class="pick">
    <Select label="Profile A" items={items} bind:value={a} />
    <Select label="Profile B" items={items} bind:value={b} />
  </div>
  {#if a && a === b}
    <EmptyState title="Pick two different profiles." body="The comparison shows what one profile has that the other does not." />
  {:else if failed}
    <p role="alert">The profiles could not be compared.</p>
  {:else if !diff}
    <div aria-busy="true" aria-label="Comparing profiles"><Skeleton lines={5} /></div>
  {:else}
    {@const d = diff}
    <div class="sections">
      <!-- Each profile's own mods sit under its picker; a kind of difference with nothing in it
           takes one line, not a section. -->
      <div class="pair">
        <Section title="Mods only in {nameOf(a)}">
          {#if d.only_in_a.length === 0}{@render none()}{:else}
            <ul class="list" aria-label="Mods only in {nameOf(a)}">
              {#each d.only_in_a as id (id)}<li>{modName(id)}</li>{/each}
            </ul>
          {/if}
        </Section>
        <Section title="Mods only in {nameOf(b)}">
          {#if d.only_in_b.length === 0}{@render none()}{:else}
            <ul class="list" aria-label="Mods only in {nameOf(b)}">
              {#each d.only_in_b as id (id)}<li>{modName(id)}</li>{/each}
            </ul>
          {/if}
        </Section>
      </div>
      {#if d.priority_diffs.length > 0}
        <Section title="Priority differences" description="Mods are loaded in order of priority, lowest number first. Each line reads {nameOf(a)} / {nameOf(b)}.">
          <ul class="list" aria-label="Priority differences">
            {#each d.priority_diffs as [id, pa, pb] (id)}
              <li><span>{modName(id)}</span><span class="mono">{pa} / {pb}</span></li>
            {/each}
          </ul>
        </Section>
      {/if}
      {#if d.setting_diffs.length > 0}
        <Section title="Setting differences">
          <ul class="list" aria-label="Setting differences">
            {#each d.setting_diffs as [id, option, va, vb] (`${id}:${option}`)}
              <li><span>{modName(id)}: {option}</span><span class="mono">{va} / {vb}</span></li>
            {/each}
          </ul>
        </Section>
      {/if}
      {#if d.priority_diffs.length === 0 && d.setting_diffs.length === 0}
        <p class="qm-muted none">No priority or setting differences.</p>
      {:else if d.priority_diffs.length === 0}
        <p class="qm-muted none">No priority differences.</p>
      {:else if d.setting_diffs.length === 0}
        <p class="qm-muted none">No setting differences.</p>
      {/if}
    </div>
  {/if}
</Dialog>

<style>
  .pick {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--qm-space-3);
    margin-bottom: var(--qm-space-4);
  }

  .sections {
    display: flex;
    flex-direction: column;
    gap: var(--qm-gap-group);
  }

  /* Side by side, the second section needs no line above it. */
  .pair > :global(.qm-section + .qm-section) {
    padding-top: 0;
    border-top: 0;
  }

  /* The same two columns as the pickers above. */
  .pair {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    align-items: start;
    gap: var(--qm-space-3);
  }

  .list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--qm-text-13);
  }

  .list li {
    display: flex;
    justify-content: space-between;
    gap: var(--qm-space-3);
    padding: var(--qm-space-1) 0;
    border-bottom: 1px solid var(--qm-border);
  }

  .list li:last-child {
    border-bottom: 0;
  }

  .mono {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  .none {
    margin: 0;
    font-size: var(--qm-text-13);
  }
</style>
