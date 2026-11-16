//! Build-time colouring for the wiki's code blocks: TOML, Lua, JSON and shell. Output is the
//! escaped source with `<span class="…">` around tokens, so the page needs no script. Classes:
//! `c` comment, `s` string, `n` number, `k` keyword, `b` constant, `a` key, `h` table header.
//!
//! The scanners only stop on ASCII bytes, so every slice falls on a character boundary.
use crate::site::esc;

#[derive(Default)]
struct Out {
    html: String,
}

impl Out {
    fn plain(&mut self, s: &str) {
        self.html.push_str(&esc(s));
    }

    fn span(&mut self, class: &str, s: &str) {
        if s.is_empty() {
            return;
        }
        self.html.push_str("<span class=\"");
        self.html.push_str(class);
        self.html.push_str("\">");
        self.html.push_str(&esc(s));
        self.html.push_str("</span>");
    }
}

/// `code` as HTML. An unknown `lang` is escaped and left uncoloured.
pub fn highlight(lang: &str, code: &str) -> String {
    let mut o = Out::default();
    match lang {
        "toml" => toml(code, &mut o),
        "lua" => lua(code, &mut o),
        "json" => json(code, &mut o),
        "bash" | "sh" | "shell" | "powershell" | "ps1" => shell(code, &mut o),
        _ => o.plain(code),
    }
    o.html
}

/// The index of the end of the line `i` is on (the `\n` itself, or the end of `src`).
fn eol(src: &str, i: usize) -> usize {
    src[i..].find('\n').map_or(src.len(), |p| i + p)
}

/// The length in bytes of the character at `i`.
fn char_len(src: &str, i: usize) -> usize {
    src[i..].chars().next().map_or(1, char::len_utf8)
}

/// The end of the run starting at `i` whose bytes all pass `keep`.
fn run(src: &str, i: usize, keep: impl Fn(u8) -> bool) -> usize {
    let b = src.as_bytes();
    let mut j = i;
    while j < b.len() && keep(b[j]) {
        j += 1;
    }
    j
}

/// The end of a one-line quoted string opening at `i`. `escapes` lets a backslash hide a quote.
fn quoted_end(src: &str, i: usize, escapes: bool) -> usize {
    let b = src.as_bytes();
    let q = b[i];
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' if escapes => j += 1,
            b'\n' => return j,
            c if c == q => return j + 1,
            _ => {}
        }
        j += 1;
    }
    b.len()
}

/// The end of a block opening at `i` and closing with `close` (searched from `from`).
fn block_end(src: &str, from: usize, close: &str) -> usize {
    src[from..]
        .find(close)
        .map_or(src.len(), |p| from + p + close.len())
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// A number token: digits and the letters, dots and signs that continue one (`0x1F`, `1.5e-3`,
/// `2026-11-06T12:00:00Z`).
fn number_end(src: &str, i: usize, dates: bool) -> usize {
    let b = src.as_bytes();
    let mut j = i + 1;
    while j < b.len() {
        let c = b[j];
        let ok = is_word(c)
            || (c == b'.' && b.get(j + 1).is_some_and(u8::is_ascii_digit))
            || ((c == b'+' || c == b'-') && matches!(b[j - 1], b'e' | b'E'))
            || (dates && (c == b'-' || c == b':'));
        if !ok {
            break;
        }
        j += 1;
    }
    j
}

/// The `=` that ends a TOML key on `line`, when the line is `key = value`.
fn toml_key_end(line: &str) -> Option<usize> {
    let mut quote = 0u8;
    for (i, c) in line.bytes().enumerate() {
        match (quote, c) {
            (0, b'=') => return Some(i),
            (0, b'#') => return None,
            (0, b'"' | b'\'') => quote = c,
            (q, c) if q != 0 && q == c => quote = 0,
            _ => {}
        }
    }
    None
}

fn toml(src: &str, o: &mut Out) {
    let b = src.as_bytes();
    let mut i = 0;
    // Only whitespace so far on this line, and not inside an array or inline table.
    let mut line_start = true;
    let mut depth = 0usize;
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            o.plain("\n");
            i += 1;
            line_start = true;
            continue;
        }
        if matches!(c, b' ' | b'\t' | b'\r') {
            let j = run(src, i, |c| matches!(c, b' ' | b'\t' | b'\r'));
            o.plain(&src[i..j]);
            i = j;
            continue;
        }
        if c == b'#' {
            let j = eol(src, i);
            o.span("c", &src[i..j]);
            i = j;
            continue;
        }
        if line_start && depth == 0 {
            line_start = false;
            let line = &src[i..eol(src, i)];
            if c == b'[' {
                let close = line.find(']').map_or(line.len(), |p| {
                    if line[p + 1..].starts_with(']') {
                        p + 2
                    } else {
                        p + 1
                    }
                });
                o.span("h", &line[..close]);
                i += close;
                continue;
            }
            if let Some(eq) = toml_key_end(line) {
                let key = line[..eq].trim_end();
                o.span("a", key);
                i += key.len();
                continue;
            }
        }
        line_start = false;
        match c {
            b'"' | b'\'' => {
                let triple = if c == b'"' { "\"\"\"" } else { "'''" };
                let j = if src[i..].starts_with(triple) {
                    block_end(src, i + 3, triple)
                } else {
                    quoted_end(src, i, c == b'"')
                };
                o.span("s", &src[i..j]);
                i = j;
            }
            b'[' | b'{' => {
                depth += 1;
                o.plain(&src[i..i + 1]);
                i += 1;
            }
            b']' | b'}' => {
                depth = depth.saturating_sub(1);
                o.plain(&src[i..i + 1]);
                i += 1;
            }
            b'0'..=b'9' => {
                let j = number_end(src, i, true);
                o.span("n", &src[i..j]);
                i = j;
            }
            b'+' | b'-' if b.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                let j = number_end(src, i + 1, true);
                o.span("n", &src[i..j]);
                i = j;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let j = run(src, i, |c| is_word(c) || c == b'-');
                match &src[i..j] {
                    w @ ("true" | "false") => o.span("b", w),
                    w => o.plain(w),
                }
                i = j;
            }
            _ => {
                let j = i + char_len(src, i);
                o.plain(&src[i..j]);
                i = j;
            }
        }
    }
}

const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if", "in", "local",
    "not", "or", "repeat", "return", "then", "until", "while",
];

fn lua(src: &str, o: &mut Out) {
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let rest = &src[i..];
        if rest.starts_with("--") {
            let j = if rest.starts_with("--[[") {
                block_end(src, i + 4, "]]")
            } else {
                eol(src, i)
            };
            o.span("c", &src[i..j]);
            i = j;
        } else if c == b'"' || c == b'\'' {
            let j = quoted_end(src, i, true);
            o.span("s", &src[i..j]);
            i = j;
        } else if rest.starts_with("[[") {
            let j = block_end(src, i + 2, "]]");
            o.span("s", &src[i..j]);
            i = j;
        } else if c.is_ascii_digit() {
            let j = number_end(src, i, false);
            o.span("n", &src[i..j]);
            i = j;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let j = run(src, i, is_word);
            let w = &src[i..j];
            if LUA_KEYWORDS.contains(&w) {
                o.span("k", w);
            } else if matches!(w, "true" | "false" | "nil") {
                o.span("b", w);
            } else {
                o.plain(w);
            }
            i = j;
        } else {
            let j = i + char_len(src, i);
            o.plain(&src[i..j]);
            i = j;
        }
    }
}

fn json(src: &str, o: &mut Out) {
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'"' {
            let j = quoted_end(src, i, true);
            let after = run(src, j, |c| matches!(c, b' ' | b'\t'));
            let class = if b.get(after) == Some(&b':') {
                "a"
            } else {
                "s"
            };
            o.span(class, &src[i..j]);
            i = j;
        } else if c.is_ascii_digit() || (c == b'-' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let first_digit = if c == b'-' { i + 1 } else { i };
            let j = number_end(src, first_digit, false);
            o.span("n", &src[i..j]);
            i = j;
        } else if c.is_ascii_alphabetic() {
            let j = run(src, i, is_word);
            match &src[i..j] {
                w @ ("true" | "false" | "null") => o.span("b", w),
                w => o.plain(w),
            }
            i = j;
        } else {
            let j = i + char_len(src, i);
            o.plain(&src[i..j]);
            i = j;
        }
    }
}

fn shell(src: &str, o: &mut Out) {
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let at_word_start = i == 0 || b[i - 1].is_ascii_whitespace();
        if c == b'#' && at_word_start {
            let j = eol(src, i);
            o.span("c", &src[i..j]);
            i = j;
        } else if c == b'"' || c == b'\'' {
            let j = quoted_end(src, i, c == b'"');
            o.span("s", &src[i..j]);
            i = j;
        } else {
            let j = i + char_len(src, i);
            o.plain(&src[i..j]);
            i = j;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The highlighted HTML with the spans removed and the entities undone.
    pub fn text_of(html: &str) -> String {
        let mut out = String::new();
        let mut rest = html;
        while let Some(p) = rest.find('<') {
            out.push_str(&rest[..p]);
            rest = &rest[p + rest[p..].find('>').unwrap() + 1..];
        }
        out.push_str(rest);
        out.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&amp;", "&")
    }

    #[test]
    fn toml_tokens() {
        let src = "# note\n[[weapons]]  # trailing\nname = \"M1 <Garand>\"\ndamage = 12.5\nauto = true\npayload = '''\nlocal x = 1 -- not a comment here\n'''\nlist = [\n  \"a\", # why\n  2,\n]\n[units.\"ß\"]\n";
        let out = highlight("toml", src);
        assert_eq!(text_of(&out), src);
        for want in [
            "<span class=\"c\"># note</span>",
            "<span class=\"h\">[[weapons]]</span>",
            "<span class=\"c\"># trailing</span>",
            "<span class=\"a\">name</span> = <span class=\"s\">&quot;M1 &lt;Garand&gt;&quot;</span>",
            "<span class=\"n\">12.5</span>",
            "<span class=\"b\">true</span>",
            "<span class=\"s\">&#39;&#39;&#39;\nlocal x = 1 -- not a comment here\n&#39;&#39;&#39;</span>",
            "<span class=\"s\">&quot;a&quot;</span>, <span class=\"c\"># why</span>",
            "<span class=\"h\">[units.&quot;ß&quot;]</span>",
        ] {
            assert!(out.contains(want), "{want} is missing from {out}");
        }
        // An array element is not a key, and a header inside an array is not a header.
        assert!(!out.contains("<span class=\"a\">  "), "{out}");
    }

    #[test]
    fn lua_tokens() {
        let src = "-- say hi\nlocal function hi(n)\n  if n == nil then return \"it's\" .. 0x1F end --[[ block\n  still ]] return [[long\nstring]]\nend\n";
        let out = highlight("lua", src);
        assert_eq!(text_of(&out), src);
        for want in [
            "<span class=\"c\">-- say hi</span>",
            "<span class=\"k\">local</span> <span class=\"k\">function</span> hi(n)",
            "<span class=\"b\">nil</span>",
            "<span class=\"s\">&quot;it&#39;s&quot;</span>",
            "<span class=\"n\">0x1F</span>",
            "<span class=\"c\">--[[ block\n  still ]]</span>",
            "<span class=\"s\">[[long\nstring]]</span>",
        ] {
            assert!(out.contains(want), "{want} is missing from {out}");
        }
    }

    #[test]
    fn json_and_shell_tokens() {
        let src = "{ \"id\": \"example_mod\", \"n\": -1.5e3, \"on\": true, \"esc\": \"a\\\"b\" }\n";
        let out = highlight("json", src);
        assert_eq!(text_of(&out), src);
        assert!(out.contains("<span class=\"a\">&quot;id&quot;</span>: <span class=\"s\">&quot;example_mod&quot;</span>"), "{out}");
        assert!(
            out.contains("<span class=\"n\">-1.5e3</span>")
                && out.contains("<span class=\"b\">true</span>")
        );
        assert!(
            out.contains("<span class=\"s\">&quot;a\\&quot;b&quot;</span>"),
            "{out}"
        );

        let src = "qm pack --out \"My Mod.zip\" # build it\necho a#b\n";
        let out = highlight("bash", src);
        assert_eq!(text_of(&out), src);
        assert!(out.contains(
            "<span class=\"s\">&quot;My Mod.zip&quot;</span> <span class=\"c\"># build it</span>"
        ));
        assert!(out.contains("echo a#b"));
    }

    #[test]
    fn unknown_language_is_only_escaped() {
        assert_eq!(
            highlight("", "a < b && \"c\"é"),
            "a &lt; b &amp;&amp; &quot;c&quot;é"
        );
        // Unfinished tokens end at the end of the text instead of panicking.
        for (lang, src) in [
            ("toml", "a = \"open"),
            ("toml", "a = '''open"),
            ("lua", "--[[ open"),
            ("lua", "x = [[open"),
            ("json", "\"open\\"),
            ("toml", "[open"),
        ] {
            assert_eq!(text_of(&highlight(lang, src)), src);
        }
    }
}
