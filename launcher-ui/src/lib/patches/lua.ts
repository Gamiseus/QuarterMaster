/**
 * A small Lua tokenizer for read-only highlighting. It only labels text; the caller renders each
 * token as a text node in a span, so patch content never becomes HTML.
 */
export type LuaToken = { kind: "keyword" | "string" | "comment" | "number" | "plain"; text: string };

const KEYWORDS = new Set([
  "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
  "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
]);

/** `[[`, `[=[`, ... at `i`: the closing bracket, or null. */
function longBracket(src: string, i: number): string | null {
  const m = /^\[(=*)\[/.exec(src.slice(i, i + 64));
  return m ? `]${m[1]}]` : null;
}

export function tokenizeLua(src: string): LuaToken[] {
  const out: LuaToken[] = [];
  const push = (kind: LuaToken["kind"], text: string) => {
    const last = out[out.length - 1];
    if (kind === "plain" && last?.kind === "plain") last.text += text;
    else out.push({ kind, text });
  };
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (src.startsWith("--", i)) {
      const close = longBracket(src, i + 2);
      let end: number;
      if (close) {
        const at = src.indexOf(close, i + 2);
        end = at === -1 ? src.length : at + close.length;
      } else {
        const nl = src.indexOf("\n", i);
        end = nl === -1 ? src.length : nl;
      }
      push("comment", src.slice(i, end));
      i = end;
    } else if (c === '"' || c === "'") {
      let j = i + 1;
      while (j < src.length && src[j] !== c && src[j] !== "\n") j += src[j] === "\\" ? 2 : 1;
      const end = Math.min(src.length, j + 1);
      push("string", src.slice(i, end));
      i = end;
    } else if (c === "[" && longBracket(src, i)) {
      const close = longBracket(src, i) as string;
      const at = src.indexOf(close, i);
      const end = at === -1 ? src.length : at + close.length;
      push("string", src.slice(i, end));
      i = end;
    } else if (/[0-9]/.test(c) || (c === "." && /[0-9]/.test(src[i + 1] ?? ""))) {
      const m = /^(0[xX][0-9a-fA-F.]+([pP][+-]?\d+)?|\d*\.?\d+([eE][+-]?\d+)?)/.exec(src.slice(i));
      const text = m ? m[0] : c;
      push("number", text);
      i += text.length;
    } else if (/[A-Za-z_]/.test(c)) {
      const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(src.slice(i)) as RegExpExecArray;
      push(KEYWORDS.has(m[0]) ? "keyword" : "plain", m[0]);
      i += m[0].length;
    } else {
      push("plain", c);
      i++;
    }
  }
  return out;
}
