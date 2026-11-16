<script lang="ts">
  import { tokenizeLua } from "../../../lib/patches/lua";

  interface Props {
    code: string;
    /** Names the block for screen readers. */
    label: string;
  }

  let { code, label }: Props = $props();

  const tokens = $derived(tokenizeLua(code));
</script>

<!-- Tokens render as text in spans: file content is never HTML. -->
<pre class="lua" aria-label={label}><code>{#each tokens as t, i (i)}<span class={`tok-${t.kind}`}>{t.text}</span>{/each}</code></pre>

<style>
  .lua {
    margin: 0;
    overflow-x: auto;
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    white-space: pre;
  }

  .tok-keyword {
    color: var(--qm-accent-text);
    font-weight: 600;
  }

  .tok-string {
    color: var(--qm-ok);
  }

  .tok-number {
    color: var(--qm-info);
  }

  .tok-comment {
    color: var(--qm-text-muted);
    font-style: italic;
  }
</style>
