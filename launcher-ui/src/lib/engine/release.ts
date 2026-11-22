/** Pure helpers for the Release view: GitHub's new-release page and the site listing text. */

/** GitHub's new-release page for `owner/repo`, with the tag, title and notes filled in. */
export function releasePageUrl(repository: string, version: string, name: string, lines: string[]): string {
  const params = new URLSearchParams({
    tag: `v${version}`,
    title: `${name} ${version}`,
    body: lines.map((l) => `- ${l}`).join("\n"),
  });
  return `https://github.com/${repository}/releases/new?${params.toString()}`;
}

/** The `[publish]` table that asks the site to list the mod. */
export function publishSnippet(repository: string, asset: string): string {
  return ["[publish]", `repository = ${JSON.stringify(repository || "your-name/your-repo")}`, `asset = ${JSON.stringify(asset)}`, 'summary = "One line shown in the mod list."', ""].join("\n");
}

/** The site registry file's line for the repository, and the file's name. */
export function registryText(repository: string): { file: string; text: string } {
  const owner = repository.split("/")[0] || "your-name";
  return { file: `registry/${owner}.toml`, text: `repository = ${JSON.stringify(repository || "your-name/your-repo")}\n` };
}

/** Changelog lines from a text box: one per line, list marks removed, blanks dropped. */
export function changelogLines(text: string): string[] {
  return text
    .split("\n")
    .map((l) => l.trim().replace(/^[-*]\s*/, "").trim())
    .filter((l) => l !== "");
}
