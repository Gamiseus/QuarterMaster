<script lang="ts">
  import { Button, Select, Switch, TextField } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type ConfigResetMode, type ConfigValue, type ModConfigSchema } from "../../../lib/bindings";
  import { buildForm, clampNumber, diffValues, selectValue, type FieldModel } from "../../../lib/modlist/config-form";

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

<div class="options">
  {#if fields.length === 0}
    <p class="qm-muted">This mod has no options.</p>
  {/if}
  {#each fields as f (f.key)}
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
  {/each}
  {#if fields.length > 0}
    <div class="actions">
      <Button variant="primary" size="sm" disabled={!canMutate || busy || !dirty} onclick={save}>Save</Button>
      <Button size="sm" disabled={!canMutate || busy} onclick={() => reset("defaults")}>Reset to defaults</Button>
      <Button variant="ghost" size="sm" disabled={!canMutate || busy} onclick={() => reset("safe")}>Safe reset</Button>
    </div>
  {/if}
</div>

<style>
  .options {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
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
