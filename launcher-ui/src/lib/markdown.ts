import MarkdownIt from "markdown-it";

const md = new MarkdownIt({ html: false, linkify: true });

const defaultLinkOpen =
  md.renderer.rules.link_open ?? ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));

// Links never navigate the webview; `guardLinks` opens https ones in the browser.
md.renderer.rules.link_open = (tokens, idx, options, env, self) => {
  tokens[idx].attrSet("rel", "noopener noreferrer");
  tokens[idx].attrSet("data-external", "");
  return defaultLinkOpen(tokens, idx, options, env, self);
};

/** True when the inline token at `idx` sits inside a link (links do not nest). */
function insideLink(tokens: { type: string }[], idx: number): boolean {
  let depth = 0;
  for (let i = 0; i < idx; i++) {
    if (tokens[i].type === "link_open") depth++;
    else if (tokens[i].type === "link_close") depth--;
  }
  return depth > 0;
}

// Remote images are never loaded: render them as a link to the image, or as plain alt text
// inside an existing link (badges like `[![Build](img)](https://ci)`).
md.renderer.rules.image = (tokens, idx, options, env, self) => {
  const token = tokens[idx];
  const src = String(token.attrGet("src") ?? "");
  const alt = self.renderInlineAsText(token.children ?? [], options, env);
  const { escapeHtml } = md.utils;
  if (insideLink(tokens, idx)) return escapeHtml(alt || src);
  return `<a href="${escapeHtml(src)}" rel="noopener noreferrer" data-external="">${escapeHtml(alt || src)}</a>`;
};

/** Renders untrusted Markdown; raw HTML is escaped. */
export function renderMarkdown(source: string): string {
  return md.render(source);
}
