<script lang="ts" module>
  export interface EditorProblem {
    from: number;
    to: number;
    severity: "error" | "warning" | "info";
    message: string;
  }

  /** Lines (1-based, inclusive) drawn with a class: `chosen`, `mod`, `mine` or `changed`. */
  export interface LineMark {
    from: number;
    to: number;
    kind: "chosen" | "mod" | "mine" | "changed";
    /** Shown when the pointer rests on the line. */
    title?: string;
  }
</script>

<script lang="ts">
  import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap, type CompletionSource } from "@codemirror/autocomplete";
  import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
  import { json } from "@codemirror/lang-json";
  import { markdown } from "@codemirror/lang-markdown";
  import { bracketMatching, HighlightStyle, indentOnInput, StreamLanguage, syntaxHighlighting } from "@codemirror/language";
  import { lua } from "@codemirror/legacy-modes/mode/lua";
  import { toml } from "@codemirror/legacy-modes/mode/toml";
  import { lintGutter, setDiagnostics } from "@codemirror/lint";
  import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
  import { Compartment, EditorSelection, EditorState, RangeSetBuilder, type Extension } from "@codemirror/state";
  import {
    Decoration,
    type DecorationSet,
    drawSelection,
    EditorView,
    highlightActiveLine,
    highlightActiveLineGutter,
    keymap,
    lineNumbers,
  } from "@codemirror/view";
  import { tags } from "@lezer/highlight";
  import { onDestroy, onMount, untrack } from "svelte";
  import type { EditorLanguage } from "../lib/engine/text";

  interface Props {
    /** The text to show. A new `docKey` replaces the whole document with it (open, reload). */
    text: string;
    docKey: string;
    language: EditorLanguage;
    label: string;
    readOnly?: boolean;
    problems?: EditorProblem[];
    complete?: CompletionSource;
    /** Moves the cursor there and scrolls it into view; a new `seq` repeats the jump. */
    jump?: { offset: number; seq: number } | null;
    onchange?: (text: string) => void;
    onsave?: () => void;
    /** Lines drawn with a background, such as the lines a patch anchors on. */
    lineMarks?: LineMark[];
    /** The lines the selection covers, each time it changes (1-based, inclusive). */
    onselectlines?: (range: { from: number; to: number }) => void;
  }

  let {
    text,
    docKey,
    language,
    label,
    readOnly = false,
    problems = [],
    complete,
    jump = null,
    onchange,
    onsave,
    lineMarks = [],
    onselectlines,
  }: Props = $props();

  let host: HTMLDivElement;
  let view: EditorView | null = null;
  const lang = new Compartment();
  const editable = new Compartment();
  const completion = new Compartment();
  const marks = new Compartment();
  /** Set while the document is replaced from outside, so no change is reported back. */
  let replacing = false;

  const highlight = HighlightStyle.define([
    { tag: [tags.keyword, tags.controlKeyword, tags.definitionKeyword, tags.operatorKeyword], color: "var(--qm-accent-text)" },
    { tag: [tags.string, tags.special(tags.string)], color: "var(--qm-ok)" },
    { tag: [tags.number, tags.bool, tags.null, tags.atom], color: "var(--qm-info)" },
    { tag: [tags.comment, tags.lineComment, tags.blockComment], color: "var(--qm-text-muted)", fontStyle: "italic" },
    { tag: [tags.propertyName, tags.attributeName, tags.definition(tags.variableName)], color: "var(--qm-warn)" },
    { tag: [tags.heading], color: "var(--qm-accent-text)", fontWeight: "700" },
    { tag: [tags.meta, tags.bracket], color: "var(--qm-text-muted)" },
  ]);

  // Colours come from the launcher's tokens, so every theme, colour vision mode and contrast applies.
  const theme = EditorView.theme({
    "&": { height: "100%", color: "var(--qm-text)", backgroundColor: "var(--qm-canvas)", fontSize: "var(--qm-text-13)" },
    ".cm-scroller": { fontFamily: "var(--qm-font-mono)", lineHeight: "1.55" },
    ".cm-content": { caretColor: "var(--qm-accent)" },
    ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--qm-accent)" },
    "&.cm-focused": { outline: "none" },
    ".cm-gutters": { backgroundColor: "var(--qm-panel)", color: "var(--qm-text-muted)", borderRight: "1px solid var(--qm-border)" },
    ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--qm-accent) 7%, transparent)" },
    ".cm-activeLineGutter": { backgroundColor: "color-mix(in srgb, var(--qm-accent) 12%, transparent)", color: "var(--qm-text)" },
    "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, ::selection": {
      backgroundColor: "color-mix(in srgb, var(--qm-accent) 30%, transparent)",
    },
    ".cm-selectionMatch": { backgroundColor: "color-mix(in srgb, var(--qm-info) 18%, transparent)" },
    ".cm-matchingBracket": { outline: "1px solid var(--qm-border-strong)", backgroundColor: "transparent" },
    ".cm-tooltip": { backgroundColor: "var(--qm-popover)", color: "var(--qm-text)", border: "1px solid var(--qm-border-strong)", borderRadius: "var(--qm-radius-sm)" },
    ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: "color-mix(in srgb, var(--qm-accent) 25%, transparent)", color: "var(--qm-text)" },
    ".cm-completionDetail": { color: "var(--qm-text-muted)", fontStyle: "normal", marginLeft: "0.75em" },
    ".cm-completionInfo": { maxWidth: "36ch" },
    ".cm-diagnostic-error": { borderLeftColor: "var(--qm-danger)" },
    ".cm-diagnostic-warning": { borderLeftColor: "var(--qm-warn)" },
    ".cm-diagnostic-info": { borderLeftColor: "var(--qm-info)" },
    ".cm-lintRange-error": { backgroundImage: "none", textDecoration: "underline wavy var(--qm-danger)", textUnderlineOffset: "3px" },
    ".cm-lintRange-warning": { backgroundImage: "none", textDecoration: "underline wavy var(--qm-warn)", textUnderlineOffset: "3px" },
    ".cm-lintRange-info": { backgroundImage: "none", textDecoration: "underline dotted var(--qm-info)", textUnderlineOffset: "3px" },
    ".cm-panels": { backgroundColor: "var(--qm-panel)", color: "var(--qm-text)", borderColor: "var(--qm-border)" },
    ".cm-textfield": { backgroundColor: "var(--qm-canvas)", color: "var(--qm-text)", border: "1px solid var(--qm-border-strong)", borderRadius: "var(--qm-radius-sm)" },
    ".cm-button": { backgroundImage: "none", backgroundColor: "var(--qm-raised)", color: "var(--qm-text)", border: "1px solid var(--qm-border-strong)" },
    ".cm-line.qm-mark-chosen": { backgroundColor: "color-mix(in srgb, var(--qm-accent) 22%, transparent)", boxShadow: "inset 3px 0 0 var(--qm-accent)" },
    ".cm-line.qm-mark-mod": { backgroundColor: "color-mix(in srgb, var(--qm-info) 12%, transparent)", boxShadow: "inset 3px 0 0 var(--qm-info)" },
    ".cm-line.qm-mark-mine": { backgroundColor: "color-mix(in srgb, var(--qm-ok) 16%, transparent)", boxShadow: "inset 3px 0 0 var(--qm-ok)" },
    ".cm-line.qm-mark-changed": { backgroundColor: "color-mix(in srgb, var(--qm-warn) 10%, transparent)", boxShadow: "inset 3px 0 0 var(--qm-warn)" },
  });

  function markExtension(list: LineMark[]): Extension {
    if (list.length === 0) return [];
    const sorted = [...list].sort((a, b) => a.from - b.from);
    return EditorView.decorations.of((view): DecorationSet => {
      const doc = view.state.doc;
      const b = new RangeSetBuilder<Decoration>();
      for (const m of sorted) {
        const deco = Decoration.line({ class: `qm-mark-${m.kind}`, attributes: m.title ? { title: m.title } : undefined });
        for (let n = Math.max(1, m.from); n <= Math.min(m.to, doc.lines); n++) b.add(doc.line(n).from, doc.line(n).from, deco);
      }
      return b.finish();
    });
  }

  function languageExtension(l: EditorLanguage): Extension {
    switch (l) {
      case "lua":
        return StreamLanguage.define(lua);
      case "toml":
        return StreamLanguage.define(toml);
      case "json":
        return json();
      case "markdown":
        return markdown();
      default:
        return [];
    }
  }

  function completionExtension(source: CompletionSource | undefined): Extension {
    return source ? autocompletion({ override: [source], activateOnTyping: true }) : [];
  }

  function stateFor(doc: string): EditorState {
    return EditorState.create({
      doc,
      extensions: [
        lineNumbers(),
        highlightActiveLineGutter(),
        highlightActiveLine(),
        history(),
        drawSelection(),
        indentOnInput(),
        bracketMatching(),
        closeBrackets(),
        highlightSelectionMatches(),
        search({ top: true }),
        lintGutter(),
        syntaxHighlighting(highlight),
        theme,
        EditorState.tabSize.of(4),
        lang.of(languageExtension(language)),
        editable.of([EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)]),
        completion.of(completionExtension(complete)),
        marks.of(markExtension(lineMarks)),
        EditorView.contentAttributes.of({ "aria-label": label }),
        keymap.of([
          { key: "Mod-s", preventDefault: true, run: () => (onsave?.(), true) },
          ...closeBracketsKeymap,
          ...defaultKeymap,
          ...searchKeymap,
          ...historyKeymap,
          ...completionKeymap,
          indentWithTab,
        ]),
        EditorView.updateListener.of((u) => {
          if (u.docChanged && !replacing) onchange?.(u.state.doc.toString());
          if (u.selectionSet && !replacing && onselectlines && u.transactions.some((t) => t.isUserEvent("select") || t.isUserEvent("select.pointer"))) {
            const sel = u.state.selection.main;
            onselectlines({ from: u.state.doc.lineAt(sel.from).number, to: u.state.doc.lineAt(sel.to).number });
          }
        }),
      ],
    });
  }

  function applyProblems(list: EditorProblem[]) {
    if (!view) return;
    const len = view.state.doc.length;
    const diagnostics = list.map((p) => ({
      from: Math.min(p.from, len),
      to: Math.min(Math.max(p.to, p.from), len),
      severity: p.severity,
      message: p.message,
    }));
    view.dispatch(setDiagnostics(view.state, diagnostics));
  }

  onMount(() => {
    view = new EditorView({ state: stateFor(text), parent: host });
    applyProblems(problems);
  });

  onDestroy(() => view?.destroy());

  // A new document (another file, a reload): a fresh state, so undo history does not cross files.
  let lastKey = untrack(() => docKey);
  $effect(() => {
    const key = docKey;
    if (!view || key === lastKey) return;
    lastKey = key;
    replacing = true;
    view.setState(stateFor(untrack(() => text)));
    replacing = false;
    untrack(() => applyProblems(problems));
  });

  $effect(() => {
    const l = language;
    view?.dispatch({ effects: lang.reconfigure(languageExtension(l)) });
  });

  $effect(() => {
    const ro = readOnly;
    view?.dispatch({ effects: editable.reconfigure([EditorState.readOnly.of(ro), EditorView.editable.of(!ro)]) });
  });

  $effect(() => {
    const c = complete;
    view?.dispatch({ effects: completion.reconfigure(completionExtension(c)) });
  });

  $effect(() => {
    applyProblems(problems);
  });

  $effect(() => {
    const m = lineMarks;
    view?.dispatch({ effects: marks.reconfigure(markExtension(m)) });
  });

  $effect(() => {
    const j = jump;
    if (!view || !j) return;
    const at = Math.min(j.offset, view.state.doc.length);
    view.dispatch({ selection: EditorSelection.cursor(at), effects: EditorView.scrollIntoView(at, { y: "center" }) });
    // Only the editor scrolls; focusing must not move the screen around it.
    view.contentDOM.focus({ preventScroll: true });
  });
</script>

<div class="qm-code-editor" bind:this={host} data-language={language}></div>

<style>
  .qm-code-editor {
    height: 100%;
    min-height: 0;
    overflow: hidden;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
  }

  .qm-code-editor :global(.cm-editor) {
    height: 100%;
  }
</style>
