<script lang="ts">
  import type { ViewLine } from "../../../lib/patches/diff";

  interface Props {
    line: ViewLine;
    mode: "split" | "unified";
    /** Mod colour number (1 to 8), or 0 for none. */
    colour: number;
    /** Mod name shown on the first line of a change (empty to hide it). */
    tag: string;
    dimmed: boolean;
    /** Long lines wrap inside their column (the row is as tall as its longest side). */
    wrap?: boolean;
    onexpand: () => void;
  }

  let { line, mode, colour, tag, dimmed, wrap = false, onexpand }: Props = $props();

  const sign = $derived(line.kind === "added" ? "+" : line.kind === "removed" ? "-" : line.kind === "replaced" ? "~" : "");
  const hidden = $derived(line.fold ? line.fold.end - line.fold.start : 0);
</script>

{#if line.fold}
  <div class={["row", "fold", { changed: line.fold.changed }, line.fold.changed && colour > 0 && `c${colour}`]}>
    <button type="button" class="expand" onclick={onexpand}>
      {line.fold.changed ? `Show ${hidden} more lines of this change` : `Show ${hidden} unchanged lines`}
    </button>
  </div>
{:else}
  <div class={["row", line.kind, mode, colour > 0 && `c${colour}`, { dimmed, wrap }]}>
    {#if mode === "split"}
      <span class="n">{line.left?.n ?? ""}</span>
      <span class="text left">{line.left?.text ?? ""}</span>
      <span class="n">{line.right?.n ?? ""}</span>
      <span class="text right">{line.right?.text ?? ""}</span>
    {:else}
      <span class="n">{line.left?.n ?? ""}</span>
      <span class="n">{line.right?.n ?? ""}</span>
      <span class="sign" aria-hidden="true">{sign}</span>
      <span class="text">{(line.right ?? line.left)?.text ?? ""}</span>
    {/if}
    <span class="tag">{tag}</span>
  </div>
{/if}

<style>
  .row {
    display: grid;
    align-items: center;
    height: 100%;
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    border-left: 3px solid transparent;
  }

  .row.split {
    grid-template-columns: 6ch minmax(0, 1fr) 6ch minmax(0, 1fr) 0;
  }

  .row.unified {
    grid-template-columns: 6ch 6ch 2ch minmax(0, 1fr) 0;
  }

  .n {
    padding-right: var(--qm-space-2);
    color: var(--qm-text-muted);
    text-align: right;
  }

  .text {
    overflow: hidden;
    white-space: pre;
    text-overflow: ellipsis;
  }

  /* Wrapped: every character can end a line, so a line's height follows from its length alone. */
  .row.wrap {
    align-items: start;
  }

  .row.wrap > * {
    line-height: var(--qm-diff-row-h);
  }

  .row.wrap .text {
    height: 100%;
    white-space: break-spaces;
    word-break: break-all;
    text-overflow: clip;
  }

  /* The mod's whole name, kept at the right edge of the view while the code scrolls sideways.
     Its column has no width, so every row's code columns line up; the name lies over the end
     of the line. */
  .tag {
    position: sticky;
    right: 0;
    justify-self: end;
    width: max-content;
    padding: 0 var(--qm-space-2);
    color: var(--attr, var(--qm-text-muted));
    font-family: var(--qm-font-sans);
    font-size: var(--qm-text-11);
    font-weight: 600;
    white-space: nowrap;
  }

  .tag:not(:empty) {
    border-left: 2px solid var(--attr, var(--qm-border-strong));
    background: var(--qm-panel);
  }

  /* Changed lines: a bar in the mod's colour and a light wash; the sign and tag carry the meaning. */
  .row:is(.added, .removed, .replaced) {
    border-left-color: var(--attr, var(--qm-border-strong));
  }

  .row.added .right,
  .row.unified.added .text {
    background: color-mix(in srgb, var(--qm-ok) 12%, transparent);
  }

  .row.removed .left,
  .row.unified.removed .text {
    background: color-mix(in srgb, var(--qm-danger) 12%, transparent);
    text-decoration: line-through;
    text-decoration-color: color-mix(in srgb, var(--qm-danger) 60%, transparent);
  }

  .row.replaced .left {
    background: color-mix(in srgb, var(--qm-danger) 12%, transparent);
  }

  .row.replaced .right {
    background: color-mix(in srgb, var(--qm-ok) 12%, transparent);
  }

  .row.dimmed {
    opacity: 0.45;
  }

  .c1 { --attr: var(--qm-attr-1); }
  .c2 { --attr: var(--qm-attr-2); }
  .c3 { --attr: var(--qm-attr-3); }
  .c4 { --attr: var(--qm-attr-4); }
  .c5 { --attr: var(--qm-attr-5); }
  .c6 { --attr: var(--qm-attr-6); }
  .c7 { --attr: var(--qm-attr-7); }
  .c8 { --attr: var(--qm-attr-8); }

  .row.fold {
    display: flex;
    background: var(--qm-raised);
  }

  /* The rest of a long change: it fades out of the change's own colour. */
  .row.fold.changed {
    border-left-color: var(--attr, var(--qm-border-strong));
    background: linear-gradient(to bottom, color-mix(in srgb, var(--qm-ok) 14%, var(--qm-panel)), var(--qm-raised));
  }

  .expand {
    flex: 1;
    border: 0;
    background: none;
    color: var(--qm-accent-text);
    font: inherit;
    font-size: var(--qm-text-12);
    cursor: pointer;
  }

  .expand:focus-visible {
    outline: none;
    box-shadow: inset var(--qm-focus-ring);
  }
</style>
