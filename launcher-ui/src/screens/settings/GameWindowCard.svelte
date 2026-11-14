<script lang="ts">
  import { untrack } from "svelte";
  import { Button, FieldRow, Panel, Select, StencilHeading, TextField } from "../../components/ui";
  import { call } from "../../lib/api";
  import { commands, type WindowSettings } from "../../lib/bindings";
  import { EMPTY_WINDOW, parseWindowNumber, sameWindow, windowIsEmpty } from "../../lib/settings/window";
  import { app } from "../../lib/stores/app.svelte";
  import { game } from "../../lib/stores/game.svelte";
  import { installs } from "../../lib/stores/installs.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";

  type Tri = "" | "on" | "off";
  const tri = (v: boolean | null | undefined): Tri => (v === true ? "on" : v === false ? "off" : "");
  const bool = (t: Tri): boolean | null => (t === "on" ? true : t === "off" ? false : null);
  /** On, off, or whatever the game ships with (named, so the choice is clear). */
  const choices = (gameValue: string) => [
    { value: "" as Tri, label: `Game default (${gameValue})` },
    { value: "on" as Tri, label: "On" },
    { value: "off" as Tri, label: "Off" },
  ];
  const TYPES = [
    { value: "", label: "Game default" },
    { value: "desktop", label: "Desktop" },
    { value: "exclusive", label: "Exclusive" },
  ];

  const saved = $derived(app.settings?.window ?? EMPTY_WINDOW);
  let width = $state("");
  let height = $state("");
  let display = $state("");
  let fullscreen = $state<Tri>("");
  let fullscreenType = $state("");
  let borderless = $state<Tri>("");
  let resizable = $state<Tri>("");
  let vsync = $state<Tri>("");
  let busy = $state(false);

  function fill(w: WindowSettings) {
    width = w.width ? String(w.width) : "";
    height = w.height ? String(w.height) : "";
    display = w.display ? String(w.display) : "";
    fullscreen = tri(w.fullscreen);
    fullscreenType = w.fullscreen_type ?? "";
    borderless = tri(w.borderless);
    resizable = tri(w.resizable);
    vsync = tri(w.vsync);
  }

  // The saved values fill the form when they load or change (Apply, Reset).
  $effect(() => {
    const w = saved;
    untrack(() => fill(w));
  });

  const draft = $derived<WindowSettings>({
    width: parseWindowNumber(width),
    height: parseWindowNumber(height),
    display: parseWindowNumber(display),
    fullscreen: bool(fullscreen),
    fullscreen_type: fullscreenType === "" ? null : fullscreenType,
    borderless: bool(borderless),
    resizable: bool(resizable),
    vsync: bool(vsync),
  });
  const changed = $derived(!sameWindow(draft, saved));
  const blocked = $derived(
    !app.startupDone ? "Available once startup has finished." : game.isBusy ? "Close the game to change these." : installs.blockReason(),
  );

  async function apply(next: WindowSettings) {
    if (busy) return;
    busy = true;
    try {
      const settings = await call(commands.windowSettingsSet(next), { expect: ["invalid_input"] });
      app.setSettings(settings);
      toasts.info(
        windowIsEmpty(next)
          ? "The game window is back to the game's own values."
          : "Saved. The Game window settings mod is on in this profile.",
      );
    } catch (e) {
      toasts.error("The window settings were not saved.", e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
</script>

<Panel class="settings-card">
  <StencilHeading level={2}>Game window</StencilHeading>
  <p class="qm-muted note">
    Left on "Game default", a value stays as the game ships it. The launcher applies your values through a small mod,
    Game window settings, which it writes and turns on in the active profile. Turn that mod on in other profiles from
    the mod list. The Vanilla profile never uses it.
  </p>
  <div>
    <FieldRow label="Window size" help="Width and height in pixels. Empty: the game's 1280 by 720.">
      <span class="size">
        <TextField class="number" label="Window width" hideLabel type="number" min="320" max="16384" placeholder="1280" bind:value={width} />
        <span aria-hidden="true">x</span>
        <TextField class="number" label="Window height" hideLabel type="number" min="320" max="16384" placeholder="720" bind:value={height} />
      </span>
    </FieldRow>
    <FieldRow label="Fullscreen">
      <Select label="Fullscreen" hideLabel class="control-select" items={choices("off")} bind:value={fullscreen} />
    </FieldRow>
    <FieldRow label="Fullscreen type" help="Used when fullscreen is on. Desktop keeps the screen's own size; Exclusive changes the screen mode, and is the game's default.">
      <Select label="Fullscreen type" hideLabel class="control-select" items={TYPES} bind:value={fullscreenType} />
    </FieldRow>
    <FieldRow label="Borderless window" help="A window with no title bar or frame.">
      <Select label="Borderless window" hideLabel class="control-select" items={choices("on")} bind:value={borderless} />
    </FieldRow>
    <FieldRow label="Resizable window" help="Lets you drag the window's edges. The game draws for 1280 by 720, so other sizes may show empty edges.">
      <Select label="Resizable window" hideLabel class="control-select" items={choices("off")} bind:value={resizable} />
    </FieldRow>
    <FieldRow label="VSync" help="Matches the frame rate to the screen. Off can lower input delay and can tear.">
      <Select label="VSync" hideLabel class="control-select" items={choices("on")} bind:value={vsync} />
    </FieldRow>
    <FieldRow label="Monitor" help="1 is the main screen. Empty: the game's choice (1).">
      <TextField class="number" label="Monitor" hideLabel type="number" min="1" max="16" placeholder="1" bind:value={display} />
    </FieldRow>
  </div>
  <div class="row">
    <Button variant="primary" size="sm" disabled={!changed || busy || blocked !== null} title={blocked ?? undefined} onclick={() => apply(draft)}>
      Apply
    </Button>
    <Button size="sm" variant="ghost" disabled={windowIsEmpty(saved) || busy || blocked !== null} title={blocked ?? undefined} onclick={() => apply(EMPTY_WINDOW)}>
      Use the game's values
    </Button>
    {#if blocked}<span class="qm-muted note">{blocked}</span>{/if}
  </div>
</Panel>

<style>
  .note {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .size {
    display: inline-flex;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }
</style>
