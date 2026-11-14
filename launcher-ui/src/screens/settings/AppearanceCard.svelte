<script lang="ts">
  import { Badge, FieldRow, Panel, Select, StencilHeading, Switch } from "../../components/ui";
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
    { value: "deutan", label: "Deuteranopia", description: "Red-green, green weak. The most common; blue, yellow and orange replace green and red." },
    { value: "protan", label: "Protanopia", description: "Red-green, red weak. Like deuteranopia, with a brighter danger colour, as reds look dark." },
    { value: "tritan", label: "Tritanopia", description: "Blue-yellow. Rare; teal, pink and red replace blue and yellow." },
    { value: "mono", label: "Monochrome", description: "Greys in clear steps, for greyscale filters. Status icons do the rest." },
  ];

  const s = $derived(app.settings);
  const disabled = $derived(!app.startupDone || !s);
  const theme = $derived(normalizeTheme(s?.theme ?? "system"));
  const vision = $derived((s?.color_vision ?? "standard") as ColourVision);
  const visionHelp = $derived(VISIONS.find((v) => v.value === vision)?.description ?? "");

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
    <!-- One row, like the other settings: the list of modes is in the select, and the
         samples show the mode in use, since a pick applies at once. -->
    <FieldRow class="vision-row" label="Colour vision" help={visionHelp}>
      <Select class="control-select" label="Colour vision" hideLabel items={VISIONS} {disabled} bind:value={() => vision, pickVision} />
    </FieldRow>
    <div class="samples" data-cvd-preview={vision} aria-label="Status colours in this mode">
      <Badge tone="ok">OK</Badge>
      <Badge tone="warn">Check</Badge>
      <Badge tone="danger">Problem</Badge>
      <Badge tone="info">Info</Badge>
    </div>
    <FieldRow label="Text size" help="Small text grows most and headings least, so the layout keeps its shape.">
      <div class="scale">
        <input
          id="qm-text-scale"
          type="range"
          aria-label="Text size"
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
    </FieldRow>
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
      <span class="qm-stencil sample-heading">Field report</span>
      <span class="sample-body">Body text at this size.</span>
      <span class="qm-muted sample-small">Captions and badges.</span>
    </div>
  </div>
</Panel>

<style>
  /* The samples close the Colour vision row, so the row's own line goes. */
  :global(.qm-field-row.vision-row) {
    padding-bottom: 0;
    border-bottom: 0;
  }

  .samples {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-1);
    padding: var(--qm-space-2) 0 var(--qm-space-3);
    border-bottom: 1px solid var(--qm-border);
  }

  .scale {
    display: grid;
    grid-template-columns: 1fr 6ch;
    align-items: center;
    gap: var(--qm-space-2);
    width: calc(var(--qm-detail-w) * 0.5);
    max-width: 100%;
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
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--qm-space-1) var(--qm-space-3);
    margin-top: var(--qm-space-2);
    padding: var(--qm-space-2) var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-canvas);
  }

  .sample .sample-heading {
    font-size: var(--sample-20);
  }

  .sample .sample-body {
    font-size: var(--sample-14);
  }

  .sample .sample-small {
    font-size: var(--sample-12);
  }
</style>
