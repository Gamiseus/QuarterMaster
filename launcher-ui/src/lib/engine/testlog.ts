/** Reading a test run's log for the Mod Engine: which lines name a file of the mod. */

export interface LogLink {
  /** The mod's file, as the editor knows it. */
  path: string;
  line: number;
  /** Where the place is in the log line, for the link text. */
  start: number;
  end: number;
}

/**
 * The mod file and line a log line names, if any: `<path>.lua:<line>` whose path ends with
 * one of the mod's files (the game sees them under `Mods/<id>/`), or a module patch's
 * `[string "<name>"]:<line>`, through `modules` (module name to the mod's file).
 */
export function linkOf(text: string, files: string[], modules: Map<string, string> = new Map()): LogLink | null {
  const named = /\[string "([^"]+)"\]:(\d+)/.exec(text);
  if (named) {
    const path = modules.get(named[1]) ?? modules.get(named[1].replace(/\.lua$/, ""));
    if (path) return { path, line: Number(named[2]), start: named.index, end: named.index + named[0].length };
  }
  const re = /([A-Za-z0-9_.\\/-]+\.lua):(\d+)/g;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    const at = m[1].replace(/\\/g, "/").toLowerCase();
    const hit = files.find((f) => {
      const lower = f.toLowerCase();
      return at === lower || at.endsWith(`/${lower}`);
    });
    if (hit) return { path: hit, line: Number(m[2]), start: m.index, end: m.index + m[0].length };
  }
  return null;
}

/** Lines worth showing by default: errors, warnings, tracebacks. */
export function isProblemLine(text: string): boolean {
  return /error|warn|traceback|stack|failed|attempt to|nil value/i.test(text);
}
