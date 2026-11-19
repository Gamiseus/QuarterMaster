<script lang="ts">
  import { Button, Select, Switch, TextField } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type ConfigResetMode, type ConfigValue, type ModConfigSchema } from "../../../lib/bindings";
  import { buildForm, clampNumber, diffValues, fieldsUnder, selectValue, type FieldModel } from "../../../lib/modlist/config-form";

  type Values = Partial<Record<string, ConfigValue>>;

  interface Props {
    modId: string;
    schema: ModConfigSchema;
    saved: Values;
    canMutate: boolean;
    /** Receives the mod's saved values after a save or reset. */
    onsaved: (values: Values) => void;
  }

  let { modId, schema, saved, canMutate, onsaved }: Props = $props();

  const initial = $derived(buildForm(schema, saved));
  const original = $derived(Object.fromEntries(initial.map((f) => [f.key, f.value])) as Values);
  // Edits are local until Save; a new schema or saved set starts over.
  let fields = $derived(initial);
  /** Raw text of number inputs being edited; cleared on change (blur) and on a new form. */
  let drafts = $derived.by((): Record<string, string> => {
    void initial;
    return {};
  });
  let busy = $state(false);

  const changes = $derived(diffValues(fields, original));
  const dirty = $derived(Object.keys(changes).length > 0);

  function set(key: string, value: ConfigValue) {
    fields = fields.map((f) => (f.key === key ? { ...f, value } : f));
  }

  /** Keeps what was typed; the value follows any finite number, unclamped while typing. */
  function typeNumber(f: FieldModel, raw: string) {
    drafts = { ...drafts, [f.key]: raw };
    const n = Number(raw);
    if (raw.trim() !== "" && Number.isFinite(n)) set(f.key, n);
  }

  /** On change (blur or Enter): clamp an edited value and show it; an unedited saved value stays as is. */
  function settleNumber(f: FieldModel) {
    const value = fields.find((x) => x.key === f.key)?.value;
    if (typeof value === "number" && value !== original[f.key]) set(f.key, clampNumber(f, value));
    const next = { ...drafts };
    delete next[f.key];
    drafts = next;
  }

  async function run(action: () => Promise<Values>) {
    busy = true;
    try {
      onsaved(await action());
    } catch {
      // call() already showed the error.
    } finally {
      busy = false;
    }
  }

  const save = () => run(() => call(commands.modConfigSet(modId, changes)));
  const reset = (mode: ConfigResetMode) => run(() => call(commands.modConfigReset(modId, mode)));
</script>

<!-- One level of options; a group (submenu) opens to its own options below its heading. -->
{#snippet level(parent: string | null)}
  {#each fieldsUnder(fields, parent) as f (f.key)}
    {#if f.kind === "group"}
      <details class="group">
        <summary>
          <span class="group-label">{f.label}</span>
          {#if f.description}
            <span class="hint qm-muted">{f.description}</span>
          {/if}
        </summary>
        <div class="options nested">
          {#if fieldsUnder(fields, f.key).length === 0}
            <p class="hint qm-muted">No options to show here with the current settings.</p>
          {/if}
          {@render level(f.key)}
        </div>
      </details>
    {:else}
      <div class="field">
        {#if f.kind === "bool"}
          <Switch
            checked={f.value === true}
            label={f.label}
            disabled={!canMutate || busy}
            onCheckedChange={(v: boolean) => set(f.key, v)}
          />
        {:else if f.kind === "number"}
          <TextField
            type="number"
            label={f.label}
            bind:value={() => drafts[f.key] ?? String(f.value), (v) => typeNumber(f, v)}
            min={f.min}
            max={f.max}
            step={f.step}
            disabled={!canMutate || busy}
            onchange={() => settleNumber(f)}
          />
        {:else if f.kind === "string"}
          <TextField
            label={f.label}
            value={String(f.value)}
            disabled={!canMutate || busy}
            oninput={(e) => set(f.key, e.currentTarget.value)}
          />
        {:else if f.kind === "select"}
          <Select
            label={f.label}
            items={(f.options ?? []).map((o, i) => ({ value: o, label: f.labels?.[i] ?? o }))}
            bind:value={() => String(f.value), (v) => v !== undefined && set(f.key, selectValue(f, v))}
            disabled={!canMutate || busy}
          />
        {:else}
          <div class="readonly">
            <span class="qm-muted">{f.label}</span>
            {#if f.value === null}
              <span class="qm-muted">Not set</span>
            {:else}
              <span>{String(f.value)}</span>
            {/if}
          </div>
        {/if}
        {#if f.description}
          <p class="hint qm-muted">{f.description}</p>
        {/if}
      </div>
    {/if}
  {/each}
{/snippet}

<div class="options">
  {#if fields.length === 0}
    <p class="qm-muted">This mod has no options.</p>
  {/if}
  {@render level(null)}
  {#if fields.length > 0}
    <div class="actions">
      <Button variant="primary" size="sm" disabled={!canMutate || busy || !dirty} onclick={save}>Save</Button>
      <Button size="sm" disabled={!canMutate || busy} onclick={() => reset("defaults")}>Reset to defaults</Button>
      <Button variant="ghost" size="sm" disabled={!canMutate || busy} onclick={() => reset("safe")}>Safe reset</Button>
    </div>
  {/if}
</div>

<style>
  /* As many columns of options as fit. */
  .options {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, calc(var(--qm-text-13) * 26)), 1fr));
    gap: var(--qm-space-3) var(--qm-space-6);
    align-items: start;
  }

  .options > :is(p, .actions, .group) {
    grid-column: 1 / -1;
  }

  /* A submenu: a bordered block that opens to its own grid of options. */
  .group {
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    padding: var(--qm-space-2) var(--qm-space-3);
  }

  .group > summary {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    cursor: pointer;
  }

  .group > summary {
    list-style: none;
  }

  .group > summary::-webkit-details-marker {
    display: none;
  }

  .group-label::before {
    content: "\25B8";
    display: inline-block;
    width: 1.2em;
    transition: transform 120ms ease;
  }

  .group[open] > summary .group-label::before {
    transform: rotate(90deg);
  }

  .group > summary:focus-visible {
    outline: none;
    box-shadow: var(--qm-focus-ring);
  }

  .nested {
    margin-top: var(--qm-space-3);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  .readonly {
    display: flex;
    justify-content: space-between;
    gap: var(--qm-space-2);
  }

  .hint {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-2);
  }
</style>
