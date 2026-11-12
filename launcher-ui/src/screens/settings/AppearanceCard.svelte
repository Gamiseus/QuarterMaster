<script lang="ts">
  import { Badge, FieldRow, Panel, RadioGroup, Section, Select, StencilHeading, Switch } from "../../components/ui";
  import type { ColourVision } from "../../lib/settings/cvd";
  import { normalizeTheme, type Theme } from "../../lib/settings/form";
  import { TEXT_SCALE_MAX, TEXT_SCALE_MIN, TEXT_SCALE_STEP, clampScale, scaledSize } from "../../lib/settings/typescale";
  import { cssVars } from "../../lib/actions/cssVars";
  import { app } from "../../lib/stores/app.svelte";

  const THEMES: { value: Theme; label: string }[] = [
    { value: "system", label: "System" },
    { value: "dark", label: "Dark" },
    { value: "light", label: "Light" },
  ];

  const VISIONS: { value: ColourVision; label: string; description: string }[] = [
    { value: "standard", label: "Standard", description: "The default colours." },
    { value: "deutan", label: "Deuteranopia (red-green, green weak)", description: "The most common; blue, yellow and orange replace green and red." },
    { value: "protan", label: "Protanopia (red-green, red weak)", description: "Like deuteranopia, with a brighter danger colour, as reds look dark." },
    { value: "tritan", label: "Tritanopia (blue-yellow)", description: "Rare; teal, pink and red replace blue and yellow." },
    { value: "mono", label: "Monochrome", description: "Greys in clear steps, for greyscale filters. Status icons do the rest." },
  ];

  const s = $derived(app.settings);
  const disabled = $derived(!app.startupDone || !s);
  const theme = $derived(normalizeTheme(s?.theme ?? "system"));
  const vision = $derived((s?.color_vision ?? "standard") as ColourVision);

  /** The slider moves freely; the value is saved when it settles (change), not on every step. */
  let draftScale = $state<number | null>(null);
  const scale = $derived(draftScale ?? clampScale(s?.text_scale ?? 1));
  const percent = $derived(Math.round(scale * 1000) / 10);

  function save(patch: Parameters<typeof app.updateSettings>[0]) {
    app.updateSettings(patch).catch(() => {});
  }

  function pickTheme(v: Theme | undefined) {
    if (v && v !== s?.theme) save({ theme: v });
  }

  function pickVision(v: ColourVision | undefined) {
    if (v && v !== vision) save({ color_vision: v });
  }

  function commitScale() {
    if (draftScale === null) return;
    const next = clampScale(draftScale);
    draftScale = null;
    if (next !== s?.text_scale) save({ text_scale: next });
  }
</script>

<Panel class="settings-card">
  <StencilHeading level={2}>Appearance</StencilHeading>
  <div>
    <FieldRow label="Theme" help="System follows Windows.">
      <Select class="control-select" label="Theme" hideLabel items={THEMES} {disabled} bind:value={() => theme, pickTheme} />
    </FieldRow>
    <FieldRow label="High contrast" help="Stronger text and borders. Works with any colour mode.">
      <Switch
        label="High contrast"
        hideLabel
        {disabled}
        bind:checked={() => s?.high_contrast ?? false, (v) => save({ high_contrast: v })}
      />
    </FieldRow>
  </div>

  <Section title="Colour vision">
    <RadioGroup class="visions" label="Colour vision" hideLabel items={VISIONS} bind:value={() => vision, pickVision}>
      {#snippet extra(mode)}
        <span class="samples" data-cvd-preview={mode}>
          <Badge tone="ok">OK</Badge>
          <Badge tone="warn">Check</Badge>
          <Badge tone="danger">Problem</Badge>
          <Badge tone="info">Info</Badge>
        </span>
      {/snippet}
    </RadioGroup>
  </Section>

  <Section title="Text size">
    <div class="scale">
      <label class="scale-label" for="qm-text-scale">Text size</label>
      <input
        id="qm-text-scale"
        type="range"
        min={TEXT_SCALE_MIN}
        max={TEXT_SCALE_MAX}
        step={TEXT_SCALE_STEP}
        value={scale}
        aria-valuetext="{percent}%"
        {disabled}
        oninput={(e) => (draftScale = Number(e.currentTarget.value))}
        onchange={commitScale}
      />
      <output class="value" for="qm-text-scale">{percent}%</output>
    </div>
    <!-- The sample uses the sizes the slider would give, before it is saved. -->
    <div
      class="sample"
      aria-hidden="true"
      use:cssVars={{
        "--sample-20": `${scaledSize(20, scale)}px`,
        "--sample-14": `${scaledSize(14, scale)}px`,
        "--sample-12": `${scaledSize(12, scale)}px`,
      }}
    >
      <p class="qm-stencil sample-heading">Field report</p>
      <p class="sample-body">Small text grows most and headings least, so the layout keeps its shape.</p>
      <p class="qm-muted sample-small">Captions and badges stay readable.</p>
    </div>
  </Section>
</Panel>

<style>
  :global(.visions .qm-radio-group) {
    gap: var(--qm-space-2);
  }

  .samples {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-1);
    margin-top: var(--qm-space-1);
  }

  .scale {
    display: grid;
    grid-template-columns: auto 1fr 6ch;
    align-items: center;
    gap: var(--qm-space-3);
  }

  .scale input {
    width: 100%;
    accent-color: var(--qm-accent);
  }

  .value {
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .sample {
    margin-top: var(--qm-space-3);
    padding: var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
  }

  .sample p {
    margin: 0;
  }

  .sample .sample-heading {
    margin-bottom: var(--qm-space-1);
    font-size: var(--sample-20);
  }

  .sample .sample-body {
    font-size: var(--sample-14);
  }

  .sample .sample-small {
    font-size: var(--sample-12);
  }
</style>
