<script lang="ts">
  // Mock builds only (`/?harness=diff`): the diff view on a 10,000-line file, for the e2e
  // performance test. Every line is shown (no folding) so the list really is 10,000 rows long.
  import type { AttributedCodeLine, FileDiff } from "../bindings";
  import DiffView from "../../screens/mods/patches/DiffView.svelte";

  const LINES = 10_000;
  const original = Array.from({ length: LINES }, (_, i) => `local value_${i + 1} = ${i + 1}`);
  const lines: AttributedCodeLine[] = original.map((text, i) =>
    (i + 1) % 500 === 0
      ? { line_number: 0, text: `${text} -- tuned`, mod_id: "ai-tweaks", patch_id: `ai-tweaks:${i}`, patch_name: "Tuning" }
      : { line_number: 0, text, mod_id: null, patch_id: null, patch_name: null },
  );
  lines.forEach((l, i) => (l.line_number = i + 1));
  const diff: FileDiff = {
    original_lines: original,
    result: {
      target_file: "scripts/big.lua",
      mods_touching: ["ai-tweaks"],
      records: [],
      attributed_lines: lines,
      original_line_count: LINES,
      patched_line_count: LINES,
    },
  };
</script>

<main class="harness">
  <DiffView {diff} modNames={{ "ai-tweaks": "AI Tweaks" }} context={LINES} />
</main>

<style>
  .harness {
    height: 100vh;
    padding: var(--qm-space-4);
    background: var(--qm-canvas);
    color: var(--qm-text);
  }
</style>
