import { describe, expect, it } from "vitest";
import { renderMarkdown } from "../../src/lib/markdown";

describe("renderMarkdown", () => {
  it("escapes raw html and renders emphasis", () => {
    const html = renderMarkdown("<script>x</script>**b**");
    expect(html).toContain("<strong>b</strong>");
    expect(html).not.toContain("<script");
  });

  it("adds rel to links", () => {
    const html = renderMarkdown("see https://example.com");
    expect(html).toContain('rel="noopener noreferrer"');
  });
});
