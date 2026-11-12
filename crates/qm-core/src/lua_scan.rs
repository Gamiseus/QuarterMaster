#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
//! A small Lua scanner: for every line, the block depth and enclosing function where the line
//! starts, and whether the line holds any code (not just comments or the inside of a long
//! string). Used to check that a patch's code still lands where its author put it.
//!
//! It knows strings, long brackets, comments and the block keywords; it is not a parser, so a
//! file that does not compile gives approximate results (the compile check reports those).

/// Scan result for one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LineInfo {
    /// Open blocks (function, if, for, while, do, repeat) where the line starts.
    pub depth: u32,
    /// Index into `Scan::functions` of the innermost function where the line starts
    /// (0 is the file's main chunk).
    pub function: usize,
    /// 1-based line that opened the innermost block where the line starts (0: none).
    pub opener: usize,
    /// The line starts inside a long comment or long string.
    pub starts_in_long: bool,
    /// The line has at least one code token.
    pub code: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub lines: Vec<LineInfo>,
    /// Function names: `<main>`, `name`, `a.b:c`, or `<anonymous>`.
    pub functions: Vec<String>,
    /// Every block opened is closed, and no `end` closes a block the text did not open.
    pub balanced: bool,
    /// A `return` outside every block: the main chunk's last statement (code after it would
    /// not compile).
    pub top_level_return: bool,
}

impl Scan {
    /// The innermost function where 1-based line `line` starts.
    pub fn function_name(&self, line: usize) -> &str {
        self.lines
            .get(line.wrapping_sub(1))
            .and_then(|l| self.functions.get(l.function))
            .map(String::as_str)
            .unwrap_or("<main>")
    }
}

#[derive(Clone, Copy)]
struct Frame {
    /// Index into `functions` for function blocks, `None` for other blocks.
    function: Option<usize>,
    opener: usize,
}

const BLOCK_WORDS: [&str; 9] = ["end", "do", "then", "function", "if", "for", "while", "repeat", "until"];

struct Scanner<'a> {
    text: &'a str,
    b: &'a [u8],
    i: usize,
    line: usize,
    stack: Vec<Frame>,
    functions: Vec<String>,
    lines: Vec<LineInfo>,
    cur: LineInfo,
    in_long: bool,
    /// Set by `while` / `for`: their `do` belongs to them and opens nothing more.
    pending_do: usize,
    /// The last name read (dotted names kept together), for `local f = function`.
    last_name: String,
    /// `last_name` when a `=` followed it.
    assign_name: Option<String>,
    /// An `end` or `until` closed a block the text never opened.
    underflow: bool,
    /// A `return` outside every block.
    top_return: bool,
}

impl Scanner<'_> {
    fn at(&self, i: usize) -> Option<u8> {
        self.b.get(i).copied()
    }

    fn function(&self) -> usize {
        self.stack.iter().rev().find_map(|f| f.function).unwrap_or(0)
    }

    fn newline(&mut self) {
        self.lines.push(self.cur);
        self.line += 1;
        self.cur = LineInfo {
            depth: u32::try_from(self.stack.len()).unwrap_or(u32::MAX),
            function: self.function(),
            opener: self.stack.last().map(|f| f.opener).unwrap_or(0),
            starts_in_long: self.in_long,
            code: false,
        };
    }

    fn push(&mut self, function: Option<usize>) {
        self.stack.push(Frame { function, opener: self.line });
    }

    /// Skips a long bracket with `level` `=` signs starting at `self.i`.
    fn skip_long(&mut self, level: usize) {
        self.i += level + 2;
        self.in_long = true;
        while let Some(c) = self.at(self.i) {
            if c == b']'
                && (1..=level).all(|k| self.at(self.i + k) == Some(b'='))
                && self.at(self.i + 1 + level) == Some(b']')
            {
                self.i += level + 2;
                self.in_long = false;
                return;
            }
            self.i += 1;
            if c == b'\n' {
                self.newline();
            }
        }
        self.in_long = false;
    }

    fn word(&mut self, word: &str) {
        match word {
            "function" => {
                let name = function_name(self.b, self.text, self.i)
                    .or_else(|| self.assign_name.take())
                    .unwrap_or_else(|| "<anonymous>".into());
                self.functions.push(name);
                self.push(Some(self.functions.len() - 1));
            }
            "if" | "repeat" => self.push(None),
            "while" | "for" => {
                self.push(None);
                self.pending_do += 1;
            }
            "do" if self.pending_do > 0 => self.pending_do -= 1,
            "do" => self.push(None),
            "end" | "until" => {
                if self.stack.pop().is_none() {
                    self.underflow = true;
                }
            }
            "return" if self.stack.is_empty() => self.top_return = true,
            _ => {}
        }
        if word != "local" {
            self.last_name = word.to_string();
        }
        self.assign_name = None;
    }

    /// The end of the name that starts at `i` (letters, digits, `_`).
    fn name_end(&self, mut i: usize) -> usize {
        while self.at(i).is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_') {
            i += 1;
        }
        i
    }

    fn run(mut self) -> Scan {
        let text = self.text;
        while let Some(c) = self.at(self.i) {
            match c {
                b'\n' => {
                    self.i += 1;
                    self.newline();
                }
                b' ' | b'\t' | b'\r' => self.i += 1,
                b'-' if self.at(self.i + 1) == Some(b'-') => {
                    self.i += 2;
                    if let Some(level) = long_open(self.b, self.i) {
                        self.skip_long(level);
                    } else {
                        while self.at(self.i).is_some_and(|x| x != b'\n') {
                            self.i += 1;
                        }
                    }
                }
                b'[' if long_open(self.b, self.i).is_some() => {
                    self.cur.code = true;
                    let level = long_open(self.b, self.i).unwrap_or(0);
                    self.skip_long(level);
                }
                b'"' | b'\'' => {
                    self.cur.code = true;
                    self.i += 1;
                    while let Some(x) = self.at(self.i).filter(|&x| x != c && x != b'\n') {
                        if x == b'\\' {
                            self.i += 1;
                            // A backslash before a newline continues the string on the next line.
                            if self.at(self.i) == Some(b'\n') {
                                self.i += 1;
                                self.newline();
                                continue;
                            }
                        }
                        self.i += 1;
                    }
                    // Past the closing quote; an unclosed string stops before the newline.
                    if self.at(self.i) == Some(c) {
                        self.i += 1;
                    }
                }
                c if c.is_ascii_alphabetic() || c == b'_' => {
                    self.cur.code = true;
                    let start = self.i;
                    self.i = self.name_end(start);
                    let word = text.get(start..self.i).unwrap_or_default();
                    self.word(word);
                }
                b'=' if self.at(self.i + 1) != Some(b'=')
                    && !matches!(
                        self.i.checked_sub(1).and_then(|p| self.at(p)),
                        Some(b'=' | b'~' | b'<' | b'>')
                    ) =>
                {
                    self.cur.code = true;
                    self.assign_name = (!self.last_name.is_empty()).then(|| self.last_name.clone());
                    self.i += 1;
                }
                b'.' | b':' if self.at(self.i + 1).is_some_and(|n| n.is_ascii_alphabetic() || n == b'_') => {
                    // Keep dotted names together for `t.f = function`.
                    self.cur.code = true;
                    let start = self.i + 1;
                    let j = self.name_end(start);
                    let word = text.get(start..j).unwrap_or_default();
                    if BLOCK_WORDS.contains(&word) {
                        // Not valid Lua as a field name; read it as the keyword.
                        self.i = start;
                        continue;
                    }
                    self.last_name = format!("{}{}{}", self.last_name, c as char, word);
                    self.i = j;
                }
                _ => {
                    self.cur.code = true;
                    if c != b')' && c != b']' {
                        self.last_name.clear();
                    }
                    self.assign_name = None;
                    self.i += 1;
                }
            }
        }
        if !text.is_empty() && !text.ends_with('\n') {
            self.lines.push(self.cur);
        }
        let balanced = !self.underflow && self.stack.is_empty();
        Scan {
            lines: self.lines,
            functions: self.functions,
            balanced,
            top_level_return: self.top_return,
        }
    }
}

/// Scans `text`; `lines` has one entry per line as `str::lines` counts them. Never panics.
pub fn scan(text: &str) -> Scan {
    Scanner {
        text,
        b: text.as_bytes(),
        i: 0,
        line: 1,
        stack: Vec::new(),
        functions: vec!["<main>".to_string()],
        lines: Vec::new(),
        cur: LineInfo::default(),
        in_long: false,
        pending_do: 0,
        last_name: String::new(),
        assign_name: None,
        underflow: false,
        top_return: false,
    }
    .run()
}

/// Whether `text` closes every block it opens and never closes one it did not open (the
/// edge rung gate, Phase C §6.1).
pub fn block_balanced(text: &str) -> bool {
    scan(text).balanced
}

/// `[[`, `[=[`, ... at `i`: the number of `=`.
fn long_open(b: &[u8], i: usize) -> Option<usize> {
    if b.get(i) != Some(&b'[') {
        return None;
    }
    let mut j = i + 1;
    while b.get(j) == Some(&b'=') {
        j += 1;
    }
    (b.get(j) == Some(&b'[')).then_some(j - i - 1)
}

/// The name after `function` at `i` (`name`, `a.b`, `a:b`), or `None` for `function (`.
fn function_name(b: &[u8], text: &str, mut i: usize) -> Option<String> {
    while b.get(i).is_some_and(|&c| c == b' ' || c == b'\t') {
        i += 1;
    }
    let start = i;
    while b
        .get(i)
        .is_some_and(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c == b':')
    {
        i += 1;
    }
    (i > start).then(|| text.get(start..i).map(str::to_string)).flatten()
}

/// What a token is (`tokens`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// A name or a keyword.
    Name,
    Number,
    /// A quoted string, quotes included.
    String,
    /// A long string (`[[...]]`, `[==[...]==]`), brackets included.
    LongString,
    /// An operator or punctuation; also any byte that starts no other token.
    Op,
}

/// One token of a Lua text. Comments are not tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// Byte range in the text.
    pub start: usize,
    pub end: usize,
    /// 1-based line where the token starts.
    pub line: usize,
}

impl Token {
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        src.get(self.start..self.end).unwrap_or("")
    }
}

const OPS3: [&[u8]; 1] = [b"..."];
const OPS2: [&[u8]; 9] = [b"..", b"==", b"~=", b"<=", b">=", b"::", b"<<", b">>", b"//"];

/// Skips a long bracket of `level` at `i`; returns the index after it (or the text's end)
/// and counts the newlines it passes.
fn skip_long_at(b: &[u8], mut i: usize, level: usize, line: &mut usize) -> usize {
    i += level + 2;
    while let Some(&c) = b.get(i) {
        if c == b']'
            && (1..=level).all(|k| b.get(i + k) == Some(&b'='))
            && b.get(i + 1 + level) == Some(&b']')
        {
            return i + level + 2;
        }
        if c == b'\n' {
            *line += 1;
        }
        i += 1;
    }
    i
}

/// The tokens of `text`, LuaJIT syntax: long brackets of any level, `0x` and exponent
/// numbers with `LL`, `ULL` and `i` suffixes, `\z` in strings, `goto` and `::labels::`.
/// Approximate on text that does not compile. Never panics.
pub fn tokens(text: &str) -> Vec<Token> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    while let Some(&c) = b.get(i) {
        let start = i;
        let start_line = line;
        let kind = match c {
            b'\n' => {
                i += 1;
                line += 1;
                continue;
            }
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => {
                i += 1;
                continue;
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                i += 2;
                if let Some(level) = long_open(b, i) {
                    i = skip_long_at(b, i, level, &mut line);
                } else {
                    while b.get(i).is_some_and(|&x| x != b'\n') {
                        i += 1;
                    }
                }
                continue;
            }
            b'[' if long_open(b, i).is_some() => {
                let level = long_open(b, i).unwrap_or(0);
                i = skip_long_at(b, i, level, &mut line);
                TokenKind::LongString
            }
            b'"' | b'\'' => {
                i += 1;
                while let Some(&x) = b.get(i) {
                    if x == c || x == b'\n' {
                        break;
                    }
                    i += 1;
                    if x == b'\\' {
                        match b.get(i) {
                            Some(b'\n') => {
                                line += 1;
                                i += 1;
                            }
                            // A CRLF line break after the backslash is one line break.
                            Some(b'\r') => {
                                i += 1;
                                if b.get(i) == Some(&b'\n') {
                                    line += 1;
                                    i += 1;
                                }
                            }
                            Some(b'z') => {
                                i += 1;
                                while let Some(&w) = b.get(i) {
                                    if !w.is_ascii_whitespace() {
                                        break;
                                    }
                                    if w == b'\n' {
                                        line += 1;
                                    }
                                    i += 1;
                                }
                            }
                            Some(_) => i += 1,
                            None => {}
                        }
                    }
                }
                if b.get(i) == Some(&c) {
                    i += 1;
                }
                TokenKind::String
            }
            b'0'..=b'9' => {
                i = number_end(b, i);
                TokenKind::Number
            }
            b'.' if b.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                i = number_end(b, i);
                TokenKind::Number
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                while b.get(i).is_some_and(|&x| x.is_ascii_alphanumeric() || x == b'_') {
                    i += 1;
                }
                TokenKind::Name
            }
            _ => {
                let rest = b.get(i..).unwrap_or_default();
                if let Some(op) = OPS3.iter().chain(OPS2.iter()).find(|op| rest.starts_with(op)) {
                    i += op.len();
                } else {
                    i += 1;
                    // Keep a multi-byte character whole.
                    while b.get(i).is_some_and(|&x| (0x80..0xc0).contains(&x)) {
                        i += 1;
                    }
                }
                TokenKind::Op
            }
        };
        out.push(Token { kind, start, end: i, line: start_line });
    }
    out
}

/// The end of the number that starts at `i`: digits, `.`, hex digits, exponents with a
/// sign, and suffixes such as `LL`.
fn number_end(b: &[u8], mut i: usize) -> usize {
    let hex = b.get(i) == Some(&b'0') && matches!(b.get(i + 1), Some(b'x' | b'X'));
    if hex {
        i += 2;
    }
    while let Some(&x) = b.get(i) {
        if !(x.is_ascii_alphanumeric() || x == b'_' || x == b'.') {
            break;
        }
        i += 1;
        let exponent = if hex { matches!(x, b'p' | b'P') } else { matches!(x, b'e' | b'E') };
        if exponent && matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depths_functions_and_comments() {
        let src = "\
local x = 1
function love.update(dt)
  if x then
    for i = 1, 3 do
      x = x + i
    end
  end
  local f = function()
    return '--not a comment'
  end
end
--[[ a long
comment ]] y = 2
while true do break end
";
        let s = scan(src);
        assert_eq!(s.lines.len(), 14);
        let d: Vec<u32> = s.lines.iter().map(|l| l.depth).collect();
        // Depth where each line starts: a closing `end` still starts inside its block.
        assert_eq!(d, vec![0, 0, 1, 2, 3, 3, 2, 1, 2, 2, 1, 0, 0, 0]);
        assert_eq!(s.function_name(5), "love.update");
        assert_eq!(s.function_name(9), "f");
        assert_eq!(s.function_name(12), "<main>");
        assert_eq!(s.lines[4].opener, 4);
        assert!(!s.lines[11].code, "the first line of a long comment is not code");
        assert!(s.lines[12].starts_in_long);
        assert!(s.lines[12].code, "`y = 2` after the comment is code");
        let only_comment = scan("-- just a comment\nx = 1\n");
        assert!(!only_comment.lines[0].code);
        assert!(only_comment.lines[1].code);
    }

    #[test]
    fn repeat_until_and_nested_long_strings() {
        let s = scan("repeat\n  local s = [==[\n ]] still\n]==]\nuntil done\nz = 1\n");
        let d: Vec<u32> = s.lines.iter().map(|l| l.depth).collect();
        assert_eq!(d, vec![0, 1, 1, 1, 1, 0]);
        assert!(s.lines[2].starts_in_long);
        assert!(!s.lines[5].starts_in_long);
    }

    #[test]
    fn method_and_field_functions() {
        let s = scan("function Unit:move(x)\n  return x\nend\nhooks.draw = function()\n  draw()\nend\n");
        assert_eq!(s.function_name(2), "Unit:move");
        assert_eq!(s.function_name(5), "hooks.draw");
    }

    #[test]
    fn balance() {
        assert!(block_balanced("function f()\n  if x then y() end\nend\n"));
        assert!(block_balanced("x = 1\n"));
        assert!(block_balanced(""));
        // GCore #16's shape: ends inside an open `if`.
        assert!(!block_balanced("a.x = a.y\nif(spamTimer <= 0) then\n"));
        // Closes a block it did not open.
        assert!(!block_balanced("  end\nx = 1\nif y then\n"));
        assert!(!block_balanced("until done\n"));
        assert!(block_balanced("s = [[ end ]] -- end\n"));
    }

    #[test]
    fn top_level_return() {
        assert!(scan("local M = {}\nreturn M\n").top_level_return);
        assert!(scan("return function()\n  return 1\nend\n").top_level_return);
        assert!(!scan("function f()\n  return 1\nend\n").top_level_return);
        assert!(!scan("do return end\n").top_level_return);
        assert!(!scan("-- return x\ns = 'return'\nt.x = 1\n").top_level_return);
    }

    /// Random bytes, Lua-ish and not, never panic, and every scan has one entry per line.
    #[test]
    fn fuzz_never_panics() {
        let alphabet: Vec<&str> = vec![
            "end", "do", "then", "function", "if", "for", "while", "repeat", "until", "local",
            " ", "\t", "\n", "\r\n", "\"", "'", "\\", "[[", "]]", "[=[", "]=]", "[==", "--",
            "--[[", "=", "==", "~=", ".", ":", "(", ")", "a", "_b1", "é", "\u{1F600}", "[", "]",
            "x.end", "t:do", "0", "-",
        ];
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..3000 {
            let n = (next() % 40) as usize;
            let mut text = String::new();
            for _ in 0..n {
                let k = (next() % alphabet.len() as u64) as usize;
                text.push_str(alphabet[k]);
            }
            let s = scan(&text);
            assert_eq!(s.lines.len(), text.lines().count(), "{text:?}");
            let _ = block_balanced(&text);
            for i in 0..=s.lines.len() + 1 {
                let _ = s.function_name(i);
            }
            for t in tokens(&text) {
                assert!(t.start < t.end && t.end <= text.len(), "{text:?}");
                assert!(text.get(t.start..t.end).is_some(), "{text:?}");
            }
        }
    }

    fn texts(src: &str) -> Vec<(TokenKind, &str, usize)> {
        tokens(src).into_iter().map(|t| (t.kind, t.text(src), t.line)).collect()
    }

    #[test]
    fn tokens_of_luajit_text() {
        use TokenKind::*;
        let src = "local x = [==[ end ]] function\n]==] .. 'a\\'b' -- c\n--[=[ long\ncomment ]=] y";
        assert_eq!(
            texts(src),
            vec![
                (Name, "local", 1),
                (Name, "x", 1),
                (Op, "=", 1),
                (LongString, "[==[ end ]] function\n]==]", 1),
                (Op, "..", 2),
                (String, "'a\\'b'", 2),
                (Name, "y", 4),
            ]
        );
        // Numbers: hex, hex floats, exponents and the LuaJIT suffixes stay one token.
        let nums = texts("0x1F 0xe+1 1e-3 0x1.8p3 10LL 0x10ULL 3i .5");
        let n: Vec<&str> = nums.iter().map(|t| t.1).collect();
        assert_eq!(n, ["0x1F", "0xe", "+", "1", "1e-3", "0x1.8p3", "10LL", "0x10ULL", "3i", ".5"]);
        assert!(nums.iter().filter(|t| t.1 != "+").all(|t| t.0 == Number));
        // goto and labels.
        let g: Vec<&str> = texts("goto continue ::continue:: a ... b ~= c").iter().map(|t| t.1).collect();
        assert_eq!(g, ["goto", "continue", "::", "continue", "::", "a", "...", "b", "~=", "c"]);
        // `\z` skips the line break that follows; lines still count.
        let z = texts("s = 'a\\z\n   b' t");
        assert_eq!(z.last(), Some(&(Name, "t", 2)));
        // A backslash-newline continues the string.
        let cont = texts("s = \"a\\\nb\" u");
        assert_eq!(cont.last(), Some(&(Name, "u", 2)));
        // The same with a CRLF line break.
        let crlf = texts("s = \"a\\\r\nb\" u");
        assert_eq!(crlf.get(2), Some(&(String, "\"a\\\r\nb\"", 1)));
        assert_eq!(crlf.last(), Some(&(Name, "u", 2)));
        // Multi-byte characters stay whole.
        assert_eq!(texts("é")[0].1, "é");
    }
}
