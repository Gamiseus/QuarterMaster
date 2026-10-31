import MarkdownIt from "markdown-it";

const md = new MarkdownIt({ html: false, linkify: true });

const defaultLinkOpen =
  md.renderer.rules.link_open ?? ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));

md.renderer.rules.link_open = (tokens, idx, options, env, self) => {
  tokens[idx].attrSet("rel", "noopener noreferrer");
  return defaultLinkOpen(tokens, idx, options, env, self);
};

/** Renders untrusted Markdown; raw HTML is escaped. */
export function renderMarkdown(source: string): string {
  return md.render(source);
}
