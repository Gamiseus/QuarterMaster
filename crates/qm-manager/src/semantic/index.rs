//! The N10 index of one Lua source (D69, §6.9): facts from `lua_scan` tokens and block
//! tracking. It is not a parser: statements are recognised by their first tokens, and scope
//! is approximate (locals are tracked per block, upvalues included). Text that does not
//! compile gives approximate facts.

use std::collections::{HashMap, HashSet};

use qm_core::lua_scan::{tokens, Token, TokenKind};

/// A block the index tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Main,
    Function,
    If,
    Do,
    While,
    For,
    Repeat,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub kind: BlockKind,
    /// 1-based line that opened it.
    pub line: usize,
    /// The enclosing block (the main block is its own parent).
    pub parent: usize,
    /// The function it belongs to (index into `SourceIndex::functions`).
    pub function: usize,
}

#[derive(Debug, Clone)]
pub struct Function {
    /// `name`, `a.b`, `a:b`, or `<anonymous>`; the main chunk is `<main>`.
    pub name: String,
    pub line: usize,
    /// The line of its `end` (the last line for the main chunk or an unclosed function).
    pub end_line: usize,
    /// The function it is defined in.
    pub parent: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    /// `function a.b(`, `a.b = function`.
    Function,
    /// `a = {`.
    Table,
}

/// A global definition (the name's first part is not a local in scope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Def {
    pub name: String,
    pub kind: DefKind,
    pub line: usize,
    /// The function the statement runs in.
    pub function: usize,
    /// The function it defines (index into `SourceIndex::functions`).
    pub body: Option<usize>,
    /// A table literal with nothing in it (`a = {}`).
    pub empty: bool,
}

/// `a.b = ...`, `a.b.c = ...` or `function a.b(`: a field set on a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldWrite {
    /// The full name; `a:b` is written `a.b`.
    pub name: String,
    pub line: usize,
    pub function: usize,
    /// The table is a local or a parameter, not a global.
    pub local: bool,
    /// The value is a function (`function a.b(`, `a.b = function`).
    pub method: bool,
    /// The table is a local set from a table literal (`local M = {}`): the mod's own table.
    pub own_table: bool,
}

/// A call with a plain name (`f(`, `a.b(`, `a:b(`, `f "x"`, `f {`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// `a.b` or `a:b`.
    pub name: String,
    pub line: usize,
}

/// `Button.new(x, y, w, h, "label", ...)`: a click calls the global `<label>Button`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonLabel {
    pub label: String,
    pub line: usize,
}

/// `menuPage == 85`, `85 == menuPage` or `menuPage = 85`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageUse {
    pub page: String,
    pub line: usize,
}

/// How a name resolves where it is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    Global,
    /// A local, parameter or upvalue: the function that declares it (`SourceIndex::
    /// function_path`), the declaring line's text (trimmed) and its 1-based line.
    Local { function: String, decl: String, line: usize },
}

impl Binding {
    /// The same declaration (the same function and declaring text), or both global. Line
    /// numbers are not compared, so two versions of a file compare.
    pub fn same(&self, other: &Binding) -> bool {
        match (self, other) {
            (Binding::Global, Binding::Global) => true,
            (Binding::Local { function: a, decl: x, .. }, Binding::Local { function: b, decl: y, .. }) => {
                a == b && x == y
            }
            _ => false,
        }
    }
}

/// A plain name used on a watched line, with what it resolves to there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameUse {
    pub name: String,
    pub line: usize,
    pub binding: Binding,
}

/// A literal file name given to a call that writes or reads the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUse {
    pub path: String,
    pub line: usize,
    pub function: usize,
    /// `love.filesystem.write` or `append`, or `io.open` with a `w` or `a` mode. Else a read:
    /// `love.filesystem.read`, `load` or `lines`, `io.open` to read, `io.lines`.
    pub write: bool,
}

/// `key == "f9"` in an `if` or `elseif` condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCheck {
    pub key: String,
    /// The modifier keys the condition asks to be down (`love.keyboard.isDown("lctrl")`, or a
    /// local set from such calls): `alt`, `ctrl`, `gui`, `shift`, sorted.
    pub modifiers: Vec<&'static str>,
    pub line: usize,
    pub function: usize,
}

/// A copy of a global followed by a new definition of it in the same function (or one inside
/// it): a wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wrapper {
    /// Index into `SourceIndex::captures`.
    pub capture: usize,
    /// Index into `SourceIndex::defs`.
    pub def: usize,
    /// The new definition calls the copy.
    pub calls: bool,
}

/// `local x = a.b`: a local copy of a global.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub local: String,
    pub name: String,
    pub line: usize,
    pub function: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Return {
    pub line: usize,
    pub function: usize,
    /// Only `do` blocks lie between it and its function (or the main chunk): the rest of
    /// the function never runs.
    pub function_level: bool,
}

/// An `if` or `elseif` whose condition is a constant (`if(1 == 0) then`, `if false then`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guard {
    pub line: usize,
    /// The condition as written.
    pub condition: String,
    pub value: bool,
    pub function: usize,
}

/// `require "x"` or `require("x")` with a literal name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Require {
    pub name: String,
    pub line: usize,
    pub function: usize,
}

/// Where a line's first token sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LineFacts {
    /// The innermost function.
    pub function: usize,
    /// The innermost block (the branch of an `if` is its `If` block).
    pub block: usize,
    /// The guard whose never-taken branch holds the line.
    pub dead: Option<usize>,
    /// The line has a token (not only comments or the inside of a long string).
    pub code: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SourceIndex {
    /// One entry per line, as `str::lines` counts them.
    pub lines: Vec<LineFacts>,
    pub functions: Vec<Function>,
    pub blocks: Vec<Block>,
    pub defs: Vec<Def>,
    pub captures: Vec<Capture>,
    pub returns: Vec<Return>,
    pub guards: Vec<Guard>,
    pub requires: Vec<Require>,
    pub field_writes: Vec<FieldWrite>,
    pub files: Vec<FileUse>,
    pub keys: Vec<KeyCheck>,
    pub wrappers: Vec<Wrapper>,
    pub calls: Vec<Call>,
    pub buttons: Vec<ButtonLabel>,
    pub pages: Vec<PageUse>,
}

impl SourceIndex {
    /// Function `f` with the functions around it, outermost first (`love.load > <anonymous>`);
    /// `<main>` for the main chunk.
    pub fn function_path(&self, f: usize) -> String {
        let mut parts = Vec::new();
        let mut cur = f;
        for _ in 0..=self.functions.len() {
            let Some(func) = self.functions.get(cur).filter(|_| cur != 0) else {
                break;
            };
            parts.push(func.name.as_str());
            cur = func.parent;
        }
        if parts.is_empty() {
            return "<main>".to_string();
        }
        parts.reverse();
        parts.join(" > ")
    }

    /// Facts of 1-based line `n`.
    pub fn line(&self, n: usize) -> Option<&LineFacts> {
        self.lines.get(n.checked_sub(1)?)
    }

    pub fn function_name(&self, f: usize) -> &str {
        self.functions.get(f).map(|f| f.name.as_str()).unwrap_or("<main>")
    }

    /// The line of function `f`'s `end`.
    pub fn end_line(&self, f: usize) -> usize {
        self.functions.get(f).map(|f| f.end_line).unwrap_or(self.lines.len())
    }
}

const KEYWORDS: [&str; 22] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if",
    "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// Tokens after which an expression goes on (`local x = a.b` followed by one of them is not a
/// plain copy).
const GOES_ON: [&str; 22] = [
    "(", "[", "{", ".", ":", "..", "+", "-", "*", "/", "%", "^", "==", "~=", "<", ">", "<=",
    ">=", "and", "or", ",", "=",
];

fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

struct Frame {
    kind: BlockKind,
    block: usize,
    function: usize,
    /// Each local with the 1-based line that declares it.
    locals: Vec<(String, usize)>,
    /// `{` depth when the block opened: an `x = ...` deeper than this is a table field.
    brace_base: usize,
    /// The current branch of an `if` never runs.
    dead: bool,
    /// An earlier branch of this `if` always runs, so the later ones never do.
    taken: bool,
    /// The constant guard behind `dead` or `taken`.
    guard: Option<usize>,
}

struct Walker<'a> {
    src: &'a str,
    toks: Vec<Token>,
    stack: Vec<Frame>,
    out: SourceIndex,
    brace: usize,
    /// `while` and `for` whose `do` is still to come.
    pending_do: usize,
    /// The name of `x = function`, for the function that follows.
    pending_fn: Option<String>,
    /// The next `function` is `local function`.
    local_fn: bool,
    /// The def of `x = function`, for the function that follows.
    pending_def: Option<usize>,
    /// Lines with facts so far.
    filled: usize,
    /// Locals that hold a copy of a global.
    capture_locals: HashSet<String>,
    /// (capture local, function): the local is called in the function or one inside it.
    calls: HashSet<(String, usize)>,
    /// Locals set from `love.keyboard.isDown` of modifier keys, with those modifiers.
    modifier_locals: HashMap<String, Vec<&'static str>>,
    /// Locals set from a table literal (`local M = {}`).
    table_locals: HashSet<String>,
    /// Line ranges whose plain names are resolved (`index_watching`).
    watch: Vec<(usize, usize)>,
    /// Names used on watched lines.
    uses: Vec<RawUse>,
}

/// A name used on a watched line: name, line, and the declaring (function, line) of a local.
type RawUse = (String, usize, Option<(usize, usize)>);

impl<'a> Walker<'a> {
    fn text(&self, k: usize) -> &'a str {
        let src = self.src;
        self.toks.get(k).map(|t| t.text(src)).unwrap_or("")
    }

    fn kind(&self, k: usize) -> Option<TokenKind> {
        self.toks.get(k).map(|t| t.kind)
    }

    fn line_of(&self, k: usize) -> usize {
        self.toks.get(k).map(|t| t.line).unwrap_or(self.filled.max(1))
    }

    /// A name token that is not a keyword.
    fn is_name(&self, k: usize) -> bool {
        self.kind(k) == Some(TokenKind::Name) && !is_keyword(self.text(k))
    }

    /// Token `k` follows `.` or `:` (a field or method name, even when it is a keyword).
    fn is_field(&self, k: usize) -> bool {
        k > 0 && matches!(self.text(k - 1), "." | ":")
    }

    fn function(&self) -> usize {
        self.stack.last().map(|f| f.function).unwrap_or(0)
    }

    fn facts(&self) -> LineFacts {
        self.facts_in(self.stack.len())
    }

    /// Line facts with only the first `n` frames open.
    fn facts_in(&self, n: usize) -> LineFacts {
        let open = self.stack.get(..n).unwrap_or(&self.stack);
        let dead = open.iter().find(|f| f.dead).and_then(|f| f.guard);
        LineFacts {
            function: self.function(),
            block: self.stack.last().map(|f| f.block).unwrap_or(0),
            dead,
            code: false,
        }
    }

    /// Records the line facts up to token `k`'s line.
    fn touch(&mut self, k: usize) {
        let Some(line) = self.toks.get(k).map(|t| t.line) else {
            return;
        };
        if line <= self.filled {
            return;
        }
        let facts = self.facts();
        while self.filled + 1 < line {
            self.out.lines.push(facts);
            self.filled += 1;
        }
        // A line that starts by closing or switching a branch is not inside that branch.
        let closer = self.kind(k) == Some(TokenKind::Name)
            && !self.is_field(k)
            && matches!(self.text(k), "end" | "until" | "else" | "elseif");
        let first = if closer && self.stack.len() > 1 {
            LineFacts { dead: self.facts_in(self.stack.len() - 1).dead, ..facts }
        } else {
            facts
        };
        self.out.lines.push(LineFacts { code: true, ..first });
        self.filled = line;
    }

    fn is_local(&self, name: &str) -> bool {
        self.stack.iter().any(|f| f.locals.iter().any(|(l, _)| l == name))
    }

    fn declare(&mut self, name: &str, line: usize) {
        if let Some(f) = self.stack.last_mut() {
            f.locals.push((name.to_string(), line));
        }
    }

    /// The innermost declaration of local `name` in scope: (function, line).
    fn resolve(&self, name: &str) -> Option<(usize, usize)> {
        self.stack.iter().rev().find_map(|f| {
            f.locals.iter().rev().find(|(l, _)| l == name).map(|(_, line)| (f.function, *line))
        })
    }

    fn watched(&self, line: usize) -> bool {
        self.watch.iter().any(|&(a, b)| a <= line && line <= b)
    }

    fn record_use(&mut self, name: &str, line: usize) {
        if self.watched(line) {
            let at = self.resolve(name);
            self.uses.push((name.to_string(), line, at));
        }
    }

    /// A plain name at `k` on a watched line, unless it is a label or a table key.
    fn use_at(&mut self, k: usize) {
        let line = self.line_of(k);
        if !self.watched(line) {
            return;
        }
        let prev = if k > 0 { self.text(k - 1) } else { "" };
        if matches!(prev, "goto" | "::") {
            return;
        }
        let brace_base = self.stack.last().map(|f| f.brace_base).unwrap_or(0);
        if self.brace > brace_base && matches!(prev, "{" | ",") && self.text(k + 1) == "=" {
            return;
        }
        let name = self.text(k);
        self.record_use(name, line);
    }

    fn push(&mut self, kind: BlockKind, line: usize, function: usize) {
        let parent = self.stack.last().map(|f| f.block).unwrap_or(0);
        self.out.blocks.push(Block { kind, line, parent, function });
        self.stack.push(Frame {
            kind,
            block: self.out.blocks.len() - 1,
            function,
            locals: Vec::new(),
            brace_base: self.brace,
            dead: false,
            taken: false,
            guard: None,
        });
    }

    fn pop(&mut self, line: usize) {
        // The main block stays.
        if self.stack.len() <= 1 {
            return;
        }
        if let Some(f) = self.stack.pop() {
            if f.kind == BlockKind::Function {
                if let Some(func) = self.out.functions.get_mut(f.function) {
                    func.end_line = line;
                }
            }
        }
    }

    /// `a`, `a.b.c`, `a:b` from token `k`: the name and the index after it.
    fn chain(&self, k: usize, colon: bool) -> Option<(String, usize)> {
        if !self.is_name(k) {
            return None;
        }
        let mut name = self.text(k).to_string();
        let mut j = k + 1;
        while matches!(self.text(j), ".") || (colon && self.text(j) == ":") {
            if self.kind(j + 1) != Some(TokenKind::Name) {
                break;
            }
            name.push_str(self.text(j));
            name.push_str(self.text(j + 1));
            j += 2;
        }
        Some((name, j))
    }

    fn base_is_local(&self, name: &str) -> bool {
        let base = name.split(['.', ':']).next().unwrap_or(name);
        self.is_local(base)
    }

    /// The tokens of an `if` or `elseif` condition at `k`: up to `then`.
    fn condition_end(&self, k: usize) -> usize {
        let mut j = k;
        while j < self.toks.len() && j < k + 64 {
            if self.text(j) == "then" && self.kind(j) == Some(TokenKind::Name) {
                return j;
            }
            j += 1;
        }
        k
    }

    fn guard_at(&mut self, k: usize) -> Option<usize> {
        let end = self.condition_end(k + 1);
        if end <= k + 1 {
            return None;
        }
        let toks: Vec<(TokenKind, &str)> = (k + 1..end)
            .filter_map(|j| self.kind(j).map(|kind| (kind, self.text(j))))
            .collect();
        let value = constant(&toks)?;
        let (first, last) = (self.toks.get(k + 1)?, self.toks.get(end - 1)?);
        let condition = self.src.get(first.start..last.end).unwrap_or("").to_string();
        self.out.guards.push(Guard {
            line: self.line_of(k),
            condition,
            value,
            function: self.function(),
        });
        Some(self.out.guards.len() - 1)
    }

    fn function_level_return(&self) -> bool {
        for f in self.stack.iter().rev() {
            match f.kind {
                BlockKind::Do => continue,
                BlockKind::Function | BlockKind::Main => return true,
                _ => return false,
            }
        }
        true
    }

    /// `function` at `k`: its name and parameters; returns the index after `)`.
    fn function_at(&mut self, k: usize) -> usize {
        let line = self.line_of(k);
        let local = std::mem::take(&mut self.local_fn);
        let pending = self.pending_fn.take();
        let (name, mut j) = match self.chain(k + 1, true) {
            Some((name, j)) => (Some(name), j),
            None => (None, k + 1),
        };
        let pending_def = self.pending_def.take();
        let parent = self.function();
        // The index the function gets below.
        let f = self.out.functions.len();
        if let Some(name) = &name {
            let local_base = self.base_is_local(name);
            if !local {
                let base = name.split(['.', ':']).next().unwrap_or(name);
                self.record_use(base, line);
            }
            if !local && !local_base {
                self.out.defs.push(Def {
                    name: name.clone(),
                    kind: DefKind::Function,
                    line,
                    function: parent,
                    body: Some(f),
                    empty: false,
                });
            }
            if !local && name.contains(['.', ':']) {
                let base = name.split(['.', ':']).next().unwrap_or(name);
                self.out.field_writes.push(FieldWrite {
                    name: name.replace(':', "."),
                    line,
                    function: parent,
                    local: local_base,
                    method: true,
                    own_table: local_base && self.table_locals.contains(base),
                });
            }
        }
        let assigned = self.text(k.wrapping_sub(1)) == "=";
        if let (None, true, Some(d)) = (&name, assigned, pending_def) {
            if let Some(def) = self.out.defs.get_mut(d) {
                def.body = Some(f);
            }
        }
        let shown = name
            .clone()
            .or(if assigned { pending } else { None })
            .unwrap_or_else(|| "<anonymous>".to_string());
        let method = shown.contains(':');
        self.out.functions.push(Function { name: shown, line, end_line: 0, parent });
        self.push(BlockKind::Function, line, f);
        if method {
            self.declare("self", line);
        }
        if self.text(j) == "(" {
            j += 1;
            while j < self.toks.len() && self.text(j) != ")" {
                self.touch(j);
                if self.is_name(j) {
                    let p = self.text(j).to_string();
                    let at = self.line_of(j);
                    self.declare(&p, at);
                }
                j += 1;
            }
            self.touch(j);
            j += 1;
        }
        j
    }

    /// `local ...` at `k`; returns the index of the next token to walk.
    fn local_at(&mut self, k: usize) -> usize {
        if self.text(k + 1) == "function" {
            if self.is_name(k + 2) {
                let name = self.text(k + 2).to_string();
                let at = self.line_of(k + 2);
                self.table_locals.remove(&name);
                self.declare(&name, at);
            }
            self.local_fn = true;
            return k + 1;
        }
        let mut names = Vec::new();
        let mut j = k + 1;
        while self.is_name(j) {
            names.push(self.text(j).to_string());
            if self.text(j + 1) == "," {
                j += 2;
            } else {
                j += 1;
                break;
            }
        }
        if names.len() == 1 && self.text(j) == "=" {
            if self.text(j + 1) == "function" {
                self.pending_fn = names.first().cloned();
            }
            if let Some((name, end)) = self.chain(j + 1, false) {
                let ends = end >= self.toks.len()
                    || !(GOES_ON.contains(&self.text(end))
                        || matches!(self.kind(end), Some(TokenKind::String | TokenKind::LongString)));
                if ends && !self.base_is_local(&name) {
                    let local = names.first().cloned().unwrap_or_default();
                    self.capture_locals.insert(local.clone());
                    self.out.captures.push(Capture {
                        local,
                        name,
                        line: self.line_of(k),
                        function: self.function(),
                    });
                }
            }
        }
        // `local ctrl = love.keyboard.isDown("lctrl") or ...`, on one line.
        let mods = if names.len() == 1 && self.text(j) == "=" {
            let line = self.line_of(j);
            let mut end = j + 1;
            while end < self.toks.len() && end < j + 48 && self.line_of(end) == line {
                end += 1;
            }
            self.modifiers_in(j + 1, end)
        } else {
            Vec::new()
        };
        let literal = names.len() == 1 && self.text(j) == "=" && self.text(j + 1) == "{";
        let at = self.line_of(k);
        for n in &names {
            self.modifier_locals.remove(n);
            if literal {
                self.table_locals.insert(n.clone());
            } else {
                self.table_locals.remove(n);
            }
            self.declare(n, at);
        }
        if let (Some(n), false) = (names.first(), mods.is_empty()) {
            self.modifier_locals.insert(n.clone(), mods);
        }
        // The names are walked again as plain tokens; that records nothing.
        k + 1
    }

    /// A statement that starts with a name at `k`: field writes (`a.b = `, `a.x, a.y = `),
    /// `a.b = function` and `a = {`.
    fn assignment_at(&mut self, k: usize) {
        let prev = if k > 0 { self.text(k - 1) } else { "" };
        if matches!(prev, "." | ":" | "," | "local" | "function" | "goto" | "::") {
            return;
        }
        let brace_base = self.stack.last().map(|f| f.brace_base).unwrap_or(0);
        if self.brace > brace_base {
            return;
        }
        let Some((name, mut j)) = self.chain(k, false) else {
            return;
        };
        let mut more = Vec::new();
        while self.text(j) == "," {
            let Some((n, e)) = self.chain(j + 1, false) else {
                return;
            };
            more.push(n);
            j = e;
        }
        if self.text(j) != "=" || self.kind(j) != Some(TokenKind::Op) {
            return;
        }
        let (line, function) = (self.line_of(k), self.function());
        let method = more.is_empty() && self.text(j + 1) == "function";
        for t in std::iter::once(&name).chain(&more).filter(|t| t.contains('.')) {
            let local = self.base_is_local(t);
            let base = t.split('.').next().unwrap_or(t);
            let own_table = local && self.table_locals.contains(base);
            self.out.field_writes.push(FieldWrite { name: t.clone(), line, function, local, method, own_table });
        }
        if !more.is_empty() {
            return;
        }
        let kind = match self.text(j + 1) {
            "function" => {
                self.pending_fn = Some(name.clone());
                DefKind::Function
            }
            "{" => DefKind::Table,
            _ => return,
        };
        if !self.base_is_local(&name) {
            let empty = kind == DefKind::Table && self.text(j + 2) == "}";
            self.out.defs.push(Def { name, kind, line, function, body: None, empty });
            if kind == DefKind::Function {
                self.pending_def = Some(self.out.defs.len() - 1);
            }
        }
    }

    /// The text of a string literal at `k`, without its quotes or brackets.
    fn literal(&self, k: usize) -> Option<&'a str> {
        let t = self.toks.get(k)?;
        let q = t.text(self.src);
        match t.kind {
            TokenKind::String => q.get(1..q.len().checked_sub(1)?),
            TokenKind::LongString => {
                let level = q.bytes().skip(1).take_while(|&c| c == b'=').count();
                q.get(level + 2..q.len().checked_sub(level + 2)?)
            }
            _ => None,
        }
    }

    /// A call at `k` that writes or reads a literal file name, also through `pcall(f, name)`.
    fn file_call_at(&mut self, k: usize) {
        if !matches!(self.text(k), "love" | "io" | "pcall") {
            return;
        }
        let Some((name, j)) = self.chain(k, false) else {
            return;
        };
        if self.text(j) != "(" {
            return;
        }
        if name == "pcall" {
            if let Some((inner, e)) = self.chain(j + 1, false) {
                if self.text(e) == "," {
                    self.file_use(&inner, e + 1, k);
                }
            }
            return;
        }
        self.file_use(&name, j + 1, k);
    }

    /// `func` called with the file name at token `arg`.
    fn file_use(&mut self, func: &str, arg: usize, k: usize) {
        let write = match func {
            "love.filesystem.write" | "love.filesystem.append" => true,
            "love.filesystem.read" | "love.filesystem.load" | "love.filesystem.lines" | "io.lines" => false,
            "io.open" => {
                let mode = if self.text(arg + 1) == "," { self.literal(arg + 2) } else { None };
                mode.is_some_and(|m| m.contains(['w', 'a']))
            }
            _ => return,
        };
        let Some(path) = self.literal(arg).filter(|p| !p.is_empty()) else {
            return;
        };
        self.out.files.push(FileUse {
            path: path.to_string(),
            line: self.line_of(k),
            function: self.function(),
            write,
        });
    }

    /// A call of a capture local at `k` (`_orig(...)`, `pcall(_orig, ...)`): counted for every
    /// function it sits in.
    fn capture_call_at(&mut self, k: usize) {
        let name = self.text(k);
        if !self.capture_locals.contains(name) {
            return;
        }
        let called = matches!(self.text(k + 1), "(" | "{")
            || matches!(self.kind(k + 1), Some(TokenKind::String | TokenKind::LongString))
            || (k >= 2 && self.text(k - 1) == "(" && matches!(self.text(k - 2), "pcall" | "xpcall"));
        if !called || self.text(k.wrapping_sub(1)) == "local" {
            return;
        }
        let functions: Vec<usize> = self
            .stack
            .iter()
            .filter(|f| f.kind == BlockKind::Function)
            .map(|f| f.function)
            .collect();
        for f in functions {
            self.calls.insert((name.to_string(), f));
        }
    }

    /// The modifier keys asked to be down by the tokens in `from..to`: `isDown` calls with
    /// modifier key names, and locals set from them; a `not` before either does not count.
    fn modifiers_in(&self, from: usize, to: usize) -> Vec<&'static str> {
        let mut out = Vec::new();
        let mut j = from;
        while j < to {
            if self.kind(j) == Some(TokenKind::Name) && self.text(j) == "isDown" && self.text(j + 1) == "(" {
                // Back over `love.keyboard.` to see a `not` in front.
                let mut s = j;
                while s >= 2 && self.text(s - 1) == "." && self.kind(s - 2) == Some(TokenKind::Name) {
                    s -= 2;
                }
                let negated = s > 0 && self.text(s - 1) == "not";
                let mut a = j + 2;
                while a < to && self.text(a) != ")" {
                    if let Some(m) = self.literal(a).and_then(modifier) {
                        if !negated {
                            out.push(m);
                        }
                    }
                    a += 1;
                }
                j = a;
            } else if self.is_name(j) && !self.is_field(j) && self.text(j.wrapping_sub(1)) != "not" {
                if let Some(m) = self.modifier_locals.get(self.text(j)) {
                    out.extend(m.iter().copied());
                }
            }
            j += 1;
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// `key == "x"` (either way round) in the condition of the `if` or `elseif` at `k`.
    fn key_checks_at(&mut self, k: usize) {
        let end = self.condition_end(k + 1);
        if end <= k + 1 {
            return;
        }
        let is_key = |w: &Self, i: usize| {
            w.kind(i) == Some(TokenKind::Name) && w.text(i) == "key" && !w.is_field(i)
        };
        let mut found = Vec::new();
        for j in k + 2..end.saturating_sub(1) {
            if self.text(j) != "==" || self.kind(j) != Some(TokenKind::Op) {
                continue;
            }
            let lit = if is_key(self, j - 1) && self.kind(j + 1) == Some(TokenKind::String) {
                self.literal(j + 1)
            } else if is_key(self, j + 1) && self.kind(j - 1) == Some(TokenKind::String) {
                self.literal(j - 1)
            } else {
                None
            };
            if let Some(key) = lit.filter(|s| !s.is_empty()) {
                found.push((key.to_string(), self.line_of(j)));
            }
        }
        if found.is_empty() {
            return;
        }
        let modifiers = self.modifiers_in(k + 1, end);
        let function = self.function();
        for (key, line) in found {
            self.out.keys.push(KeyCheck { key, modifiers: modifiers.clone(), line, function });
        }
    }

    /// A call of a plain name at `k` (`f(`, `a.b(`, `a:b(`, `f "x"`, `f {`).
    fn call_at(&mut self, k: usize) {
        let Some((name, j)) = self.chain(k, true) else {
            return;
        };
        let called = matches!(self.text(j), "(" | "{")
            || matches!(self.kind(j), Some(TokenKind::String | TokenKind::LongString));
        if called {
            self.out.calls.push(Call { name, line: self.line_of(k) });
        }
    }

    /// `Button.new(x, y, w, h, "label", ...)` at `k`: the fifth argument, when it is a literal.
    fn button_at(&mut self, k: usize) {
        if self.text(k) != "Button" || self.text(k + 1) != "." || self.text(k + 2) != "new" || self.text(k + 3) != "(" {
            return;
        }
        let (mut depth, mut commas) = (0usize, 0usize);
        let mut j = k + 4;
        while j < self.toks.len() && j < k + 96 {
            match self.text(j) {
                "(" | "{" | "[" => depth += 1,
                ")" | "}" | "]" if depth == 0 => return,
                ")" | "}" | "]" => depth -= 1,
                "," if depth == 0 => commas += 1,
                _ if depth == 0 && commas == 4 => {
                    if self.text(j + 1) == "," || self.text(j + 1) == ")" {
                        if let Some(label) = self.literal(j).filter(|l| !l.is_empty()) {
                            let line = self.line_of(k);
                            self.out.buttons.push(ButtonLabel { label: label.to_string(), line });
                        }
                    }
                    return;
                }
                _ => {}
            }
            j += 1;
        }
    }

    /// `menuPage == 85`, `85 == menuPage` or `menuPage = 85` at `k`.
    fn page_at(&mut self, k: usize) {
        if self.text(k) != "menuPage" {
            return;
        }
        let after = matches!(self.text(k + 1), "==" | "=") && self.kind(k + 2) == Some(TokenKind::Number);
        let before = k >= 2 && self.text(k - 1) == "==" && self.kind(k - 2) == Some(TokenKind::Number);
        let lit = if after {
            k + 2
        } else if before {
            k - 2
        } else {
            return;
        };
        if let Some(n) = number(self.text(lit)).filter(|n| n.fract() == 0.0) {
            let line = self.line_of(k);
            self.out.pages.push(PageUse { page: format!("{}", n as i64), line });
        }
    }

    fn require_at(&mut self, k: usize) {
        let s = if self.text(k + 1) == "(" { k + 2 } else { k + 1 };
        let Some(name) = self.literal(s).filter(|n| !n.is_empty()) else {
            return;
        };
        self.out.requires.push(Require {
            name: name.to_string(),
            line: self.line_of(k),
            function: self.function(),
        });
    }

    /// The index, and the names used on watched lines.
    fn run(mut self, n_lines: usize) -> (SourceIndex, Vec<RawUse>) {
        self.out.functions.push(Function {
            name: "<main>".to_string(),
            line: 1,
            end_line: n_lines,
            parent: 0,
        });
        self.out.blocks.push(Block { kind: BlockKind::Main, line: 1, parent: 0, function: 0 });
        self.stack.push(Frame {
            kind: BlockKind::Main,
            block: 0,
            function: 0,
            locals: Vec::new(),
            brace_base: 0,
            dead: false,
            taken: false,
            guard: None,
        });
        let mut k = 0usize;
        while k < self.toks.len() {
            self.touch(k);
            let line = self.line_of(k);
            let text = self.text(k);
            let keyword = self.kind(k) == Some(TokenKind::Name) && !self.is_field(k);
            let mut next = k + 1;
            match (self.kind(k), text) {
                (Some(TokenKind::Op), "{") => self.brace += 1,
                (Some(TokenKind::Op), "}") => self.brace = self.brace.saturating_sub(1),
                _ if !keyword => {}
                (_, "local") => next = self.local_at(k),
                (_, "function") => next = self.function_at(k),
                (_, "if") => {
                    self.key_checks_at(k);
                    let guard = self.guard_at(k);
                    let function = self.function();
                    self.push(BlockKind::If, line, function);
                    let value = guard.and_then(|g| self.out.guards.get(g)).map(|g| g.value);
                    if let Some(f) = self.stack.last_mut() {
                        f.guard = guard;
                        f.dead = value == Some(false);
                        f.taken = value == Some(true);
                    }
                }
                (_, "elseif") => {
                    self.key_checks_at(k);
                    let in_if = self.stack.last().is_some_and(|f| f.kind == BlockKind::If);
                    let taken = self.stack.last().is_some_and(|f| f.taken);
                    if in_if && !taken {
                        let guard = self.guard_at(k);
                        let value = guard.and_then(|g| self.out.guards.get(g)).map(|g| g.value);
                        if let Some(f) = self.stack.last_mut() {
                            f.guard = guard;
                            f.dead = value == Some(false);
                            f.taken = value == Some(true);
                        }
                    } else if in_if {
                        if let Some(f) = self.stack.last_mut() {
                            f.dead = true;
                        }
                    }
                }
                (_, "else") => {
                    if let Some(f) = self.stack.last_mut().filter(|f| f.kind == BlockKind::If) {
                        f.dead = f.taken;
                        if !f.taken {
                            f.guard = None;
                        }
                    }
                }
                (_, "while") => {
                    let function = self.function();
                    self.push(BlockKind::While, line, function);
                    self.pending_do += 1;
                }
                (_, "for") => {
                    let function = self.function();
                    self.push(BlockKind::For, line, function);
                    self.pending_do += 1;
                    let mut j = k + 1;
                    while self.is_name(j) {
                        let v = self.text(j).to_string();
                        self.declare(&v, line);
                        if self.text(j + 1) == "," {
                            j += 2;
                        } else {
                            break;
                        }
                    }
                }
                (_, "do") if self.pending_do > 0 => self.pending_do -= 1,
                (_, "do") => {
                    let function = self.function();
                    self.push(BlockKind::Do, line, function);
                }
                (_, "repeat") => {
                    let function = self.function();
                    self.push(BlockKind::Repeat, line, function);
                }
                (_, "end") | (_, "until") => self.pop(line),
                (_, "return") => {
                    let function_level = self.function_level_return();
                    let function = self.function();
                    self.out.returns.push(Return { line, function, function_level });
                }
                (_, "require") => self.require_at(k),
                (Some(TokenKind::Name), t) if !is_keyword(t) => {
                    self.use_at(k);
                    self.assignment_at(k);
                    self.file_call_at(k);
                    self.capture_call_at(k);
                    self.call_at(k);
                    self.button_at(k);
                    self.page_at(k);
                }
                _ => {}
            }
            k = next.max(k + 1);
        }
        let facts = self.facts();
        while self.filled < n_lines {
            self.out.lines.push(facts);
            self.filled += 1;
        }
        self.out.lines.truncate(n_lines);
        // Functions never closed end at the last line.
        for f in &mut self.out.functions {
            if f.end_line == 0 {
                f.end_line = n_lines;
            }
        }
        self.out.wrappers = wrappers(&self.out, &self.calls);
        (self.out, self.uses)
    }
}

/// Function `outer` is `f` or encloses it (the main chunk encloses every function).
fn encloses(ix: &SourceIndex, outer: usize, mut f: usize) -> bool {
    for _ in 0..ix.functions.len() + 1 {
        if f == outer || outer == 0 {
            return true;
        }
        match ix.functions.get(f) {
            Some(func) if f != 0 => f = func.parent,
            _ => return false,
        }
    }
    false
}

/// Each global function definition with a copy of the same global taken before it, in the
/// definition's function or one around it.
fn wrappers(ix: &SourceIndex, calls: &HashSet<(String, usize)>) -> Vec<Wrapper> {
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, c) in ix.captures.iter().enumerate() {
        by_name.entry(c.name.as_str()).or_default().push(i);
    }
    let mut out = Vec::new();
    for (di, d) in ix.defs.iter().enumerate() {
        let (DefKind::Function, Some(body)) = (d.kind, d.body) else {
            continue;
        };
        for &ci in by_name.get(d.name.as_str()).into_iter().flatten() {
            let Some(c) = ix.captures.get(ci) else {
                continue;
            };
            if c.line > d.line || !encloses(ix, c.function, d.function) {
                continue;
            }
            let called = calls.contains(&(c.local.clone(), body));
            out.push(Wrapper { capture: ci, def: di, calls: called });
        }
    }
    out
}

/// The modifier a key name stands for.
fn modifier(key: &str) -> Option<&'static str> {
    match key {
        "lctrl" | "rctrl" => Some("ctrl"),
        "lshift" | "rshift" => Some("shift"),
        "lalt" | "ralt" => Some("alt"),
        "lgui" | "rgui" => Some("gui"),
        _ => None,
    }
}

fn walker<'a>(text: &'a str, watch: Vec<(usize, usize)>) -> Walker<'a> {
    Walker {
        src: text,
        toks: tokens(text),
        stack: Vec::new(),
        out: SourceIndex::default(),
        brace: 0,
        pending_do: 0,
        pending_fn: None,
        local_fn: false,
        pending_def: None,
        filled: 0,
        capture_locals: HashSet::new(),
        calls: HashSet::new(),
        modifier_locals: HashMap::new(),
        table_locals: HashSet::new(),
        watch,
        uses: Vec::new(),
    }
}

/// The index of `text`. Never panics.
pub fn index(text: &str) -> SourceIndex {
    walker(text, Vec::new()).run(text.lines().count()).0
}

/// The index of `text`, and the plain names used on the lines of `ranges` (1-based, both
/// ends included) with what each resolves to there. Scope is approximate: in `local x = x`
/// the second `x` counts as the new local.
pub fn index_watching(text: &str, ranges: &[(usize, usize)]) -> (SourceIndex, Vec<NameUse>) {
    let (ix, raw) = walker(text, ranges.to_vec()).run(text.lines().count());
    let lines: Vec<&str> = text.lines().collect();
    let uses = raw
        .into_iter()
        .map(|(name, line, at)| {
            let binding = match at {
                None => Binding::Global,
                Some((f, decl)) => Binding::Local {
                    function: ix.function_path(f),
                    decl: lines.get(decl.wrapping_sub(1)).map(|l| l.trim().to_string()).unwrap_or_default(),
                    line: decl,
                },
            };
            NameUse { name, line, binding }
        })
        .collect();
    (ix, uses)
}

/// The names used in lines `first..=last` that are declared outside them, each once (its
/// first use).
pub fn free_names(uses: &[NameUse], first: usize, last: usize) -> Vec<&NameUse> {
    let mut seen = HashSet::new();
    uses.iter()
        .filter(|u| first <= u.line && u.line <= last)
        .filter(|u| match &u.binding {
            Binding::Global => true,
            Binding::Local { line, .. } => *line < first || *line > last,
        })
        .filter(|u| seen.insert(u.name.as_str()))
        .collect()
}

/// The blocks `text` opens minus those it closes: `function`, `if`, `do` (also of `while`
/// and `for`) and `repeat` open; `end` and `until` close.
pub fn block_delta(text: &str) -> i64 {
    let toks = tokens(text);
    let mut d = 0i64;
    for (i, t) in toks.iter().enumerate() {
        if t.kind != TokenKind::Name {
            continue;
        }
        let field = i > 0 && toks.get(i - 1).is_some_and(|p| matches!(p.text(text), "." | ":"));
        if field {
            continue;
        }
        match t.text(text) {
            "function" | "if" | "do" | "repeat" => d += 1,
            "end" | "until" => d -= 1,
            _ => {}
        }
    }
    d
}

/// The tokens of `text`, as text.
pub fn token_texts(text: &str) -> Vec<&str> {
    tokens(text).iter().map(|t| t.text(text)).collect()
}

/// The conditions of the `if`, `elseif` and `while` statements in `text`, as tokens, with
/// parentheses around the whole condition left out.
pub fn conditions(text: &str) -> Vec<Vec<&str>> {
    let toks = tokens(text);
    let words: Vec<(TokenKind, &str)> = toks.iter().map(|t| (t.kind, t.text(text))).collect();
    let mut out = Vec::new();
    for (i, (kind, w)) in words.iter().enumerate() {
        let field = i > 0 && words.get(i - 1).is_some_and(|(_, p)| matches!(*p, "." | ":"));
        if *kind != TokenKind::Name || field || !matches!(*w, "if" | "elseif" | "while") {
            continue;
        }
        let stop = if *w == "while" { "do" } else { "then" };
        let Some(end) = (i + 1..words.len().min(i + 64)).find(|&j| words.get(j).is_some_and(|(_, t)| *t == stop)) else {
            continue;
        };
        let mut cond: &[(TokenKind, &str)] = words.get(i + 1..end).unwrap_or(&[]);
        while let [(_, "("), inner @ .., (_, ")")] = cond {
            if !parens_wrap(cond) {
                break;
            }
            cond = inner;
        }
        out.push(cond.iter().map(|(_, t)| *t).collect());
    }
    out
}

/// The value of a constant condition: `false`, `nil`, `true`, `not <constant>`, and
/// literal comparisons such as `1 == 0`, each with any parentheses around it.
fn constant(toks: &[(TokenKind, &str)]) -> Option<bool> {
    let mut t = toks;
    while let [(_, "("), inner @ .., (_, ")")] = t {
        if !parens_wrap(t) {
            break;
        }
        t = inner;
    }
    match t {
        [(TokenKind::Name, "false")] | [(TokenKind::Name, "nil")] => Some(false),
        [(TokenKind::Name, "true")] => Some(true),
        // `not` binds tighter than `==`: `not 1 == 0` is `(not 1) == 0`, so it takes one
        // operand only (a token, a group in parentheses or another `not`).
        [(TokenKind::Name, "not"), rest @ ..] if unary_operand(rest) => constant(rest).map(|v| !v),
        [(ka, a), (TokenKind::Op, op @ ("==" | "~=")), (kb, b)] => {
            let same = match (ka, kb) {
                (TokenKind::Number, TokenKind::Number) => number(a)? == number(b)?,
                (TokenKind::String, TokenKind::String) => unquote(a)? == unquote(b)?,
                _ => return None,
            };
            Some(if *op == "==" { same } else { !same })
        }
        _ => None,
    }
}

/// Whether `t` is the whole operand of a unary `not`.
fn unary_operand(t: &[(TokenKind, &str)]) -> bool {
    match t {
        [_] => true,
        [(_, "("), ..] => parens_wrap(t),
        [(TokenKind::Name, "not"), rest @ ..] => unary_operand(rest),
        _ => false,
    }
}

/// Whether the `(` at the start of `t` closes at its end.
fn parens_wrap(t: &[(TokenKind, &str)]) -> bool {
    let mut depth = 0usize;
    for (i, (_, s)) in t.iter().enumerate() {
        match *s {
            "(" => depth += 1,
            ")" => {
                depth = depth.saturating_sub(1);
                if depth == 0 && i + 1 < t.len() {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

fn number(s: &str) -> Option<f64> {
    let s = s.trim_end_matches(['L', 'U', 'l', 'u']);
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return i64::from_str_radix(hex, 16).ok().map(|v| v as f64);
    }
    s.parse::<f64>().ok()
}

fn unquote(s: &str) -> Option<&str> {
    s.get(1..s.len().checked_sub(1)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(defs: &[Def]) -> Vec<(&str, DefKind, usize)> {
        defs.iter().map(|d| (d.name.as_str(), d.kind, d.line)).collect()
    }

    #[test]
    fn global_definitions_and_locals() {
        let src = "\
function love.keypressed(key)
  local function helper() end
  function inner() end
end
local M = {}
function M.f() end
settings = { a = function() end }
hooks.draw = function() end
local t = {}
t.x = function() end
local function solo() end
function Unit:move(x) self.x = x end
";
        let ix = index(src);
        assert_eq!(
            names(&ix.defs),
            vec![
                ("love.keypressed", DefKind::Function, 1),
                ("inner", DefKind::Function, 3),
                ("settings", DefKind::Table, 7),
                ("hooks.draw", DefKind::Function, 8),
                ("Unit:move", DefKind::Function, 12),
            ]
        );
        // `inner` runs when love.keypressed runs.
        assert_eq!(ix.function_name(ix.defs[1].function), "love.keypressed");
        assert_eq!(ix.defs[0].function, 0);
        assert_eq!(ix.function_name(ix.lines[1].function), "love.keypressed");
        assert_eq!(ix.functions[1].end_line, 4);
    }

    #[test]
    fn captures_are_plain_copies_of_globals() {
        let src = "\
local _orig = loadModSettings
local floor = math.floor
local x = a.b(1)
local y = a or b
local s = f 'x'
local p, q = r, s
local M = {}
local g = M.f
local _k = love.keypressed
function love.keypressed(k) _k(k) end
";
        let ix = index(src);
        let caps: Vec<(&str, &str, usize)> =
            ix.captures.iter().map(|c| (c.local.as_str(), c.name.as_str(), c.line)).collect();
        assert_eq!(
            caps,
            vec![
                ("_orig", "loadModSettings", 1),
                ("floor", "math.floor", 2),
                ("_k", "love.keypressed", 9),
            ]
        );
    }

    #[test]
    fn returns_say_whether_the_rest_of_the_function_runs() {
        let src = "\
function a()
  if x then return end
  do return end
  y()
end
return M
";
        let ix = index(src);
        let r: Vec<(usize, bool, &str)> = ix
            .returns
            .iter()
            .map(|r| (r.line, r.function_level, ix.function_name(r.function)))
            .collect();
        assert_eq!(r, vec![(2, false, "a"), (3, true, "a"), (6, true, "<main>")]);
    }

    #[test]
    fn constant_false_guards_mark_their_branch() {
        let src = "\
if(1 == 0) then
  dead()
elseif x then
  live()
else
  live2()
end
if true then
  a()
else
  b()
end
if (false) then c() end
if x == 0 then d() end
if not nil then e() end
if not 1 == 0 then f() end
if not (1 == 0) then g() end
if not not false then h() end
";
        let ix = index(src);
        let g: Vec<(usize, &str, bool)> =
            ix.guards.iter().map(|g| (g.line, g.condition.as_str(), g.value)).collect();
        // `not 1 == 0` is `(not 1) == 0`: not a literal comparison, so not a guard.
        assert_eq!(
            g,
            vec![
                (1, "(1 == 0)", false),
                (8, "true", true),
                (13, "(false)", false),
                (15, "not nil", true),
                (17, "not (1 == 0)", true),
                (18, "not not false", false),
            ]
        );
        let dead: Vec<usize> = (1..=ix.lines.len())
            .filter(|&n| ix.line(n).is_some_and(|l| l.dead.is_some()))
            .collect();
        // The `elseif`, `else` and `end` lines that close a dead branch are not in it.
        assert_eq!(dead, vec![2, 11]);
    }

    #[test]
    fn requires_and_run_context() {
        let src = "\
require \"includes/directives\"
local ser = require('includes/ser')
function love.load()
  require [[late]]
end
";
        let ix = index(src);
        let r: Vec<(&str, usize, usize)> =
            ix.requires.iter().map(|r| (r.name.as_str(), r.line, r.function)).collect();
        assert_eq!(r, vec![("includes/directives", 1, 0), ("includes/ser", 2, 0), ("late", 4, 1)]);
    }

    /// LuaJIT lexing: long brackets of level 2, `0x` and `LL` numbers, `goto` and labels
    /// leave the block tracking intact.
    #[test]
    fn luajit_syntax_keeps_blocks() {
        let src = "\
local s = [==[
function fake() end
]==]
for i = 0x1, 10LL do
  if i == 0x2 then goto continue end
  ::continue::
end
function real() return 1 end
";
        let ix = index(src);
        assert_eq!(names(&ix.defs), vec![("real", DefKind::Function, 8)]);
        assert!(ix.lines.iter().all(|l| l.function == 0));
        assert!(!ix.lines[1].code, "the inside of a long string is not code");
        assert_eq!(ix.line(8).map(|l| l.block), Some(0));
        let ends: Vec<usize> = ix.functions.iter().map(|f| f.end_line).collect();
        assert_eq!(ends, vec![8, 8]);
    }

    #[test]
    fn field_writes_and_table_literals() {
        let src = "\
modSettings.a = 1
modSettings.x, modSettings.y = 1, 2
local t = {}
t.z = 3
function modSettings.f() end
function Unit:move() end
modSettings = { b = 2 }
other = {}
merged = merged or {}
x = { a = { b = 1 } }
";
        let ix = index(src);
        let w: Vec<(&str, usize, bool)> =
            ix.field_writes.iter().map(|f| (f.name.as_str(), f.line, f.local)).collect();
        assert_eq!(
            w,
            vec![
                ("modSettings.a", 1, false),
                ("modSettings.x", 2, false),
                ("modSettings.y", 2, false),
                ("t.z", 4, true),
                ("modSettings.f", 5, false),
                ("Unit.move", 6, false),
            ]
        );
        let tables: Vec<(&str, bool)> = ix
            .defs
            .iter()
            .filter(|d| d.kind == DefKind::Table)
            .map(|d| (d.name.as_str(), d.empty))
            .collect();
        assert_eq!(tables, vec![("modSettings", false), ("other", true), ("x", false)]);
    }

    #[test]
    fn file_names_written_and_read() {
        let src = "\
function save()
  local ok, data = pcall(love.filesystem.load, \"mod_settings.lua\")
  love.filesystem.write(\"mod_settings.lua\", s)
end
love.filesystem.append('log.txt', s)
local f = io.open(\"a.txt\", \"wb\")
local g = io.open(\"b.txt\")
local r = love.filesystem.read([[c.txt]])
io.open(path .. \"/x.txt\", \"w\")
";
        let ix = index(src);
        let f: Vec<(&str, usize, bool, &str)> = ix
            .files
            .iter()
            .map(|u| (u.path.as_str(), u.line, u.write, ix.function_name(u.function)))
            .collect();
        assert_eq!(
            f,
            vec![
                ("mod_settings.lua", 2, false, "save"),
                ("mod_settings.lua", 3, true, "save"),
                ("log.txt", 5, true, "<main>"),
                ("a.txt", 6, true, "<main>"),
                ("b.txt", 7, false, "<main>"),
                ("c.txt", 8, false, "<main>"),
            ]
        );
    }

    #[test]
    fn key_checks_with_their_modifiers() {
        let src = "\
function love.keypressed(key)
  local ctrl = love.keyboard.isDown(\"lctrl\") or love.keyboard.isDown(\"rctrl\")
  if key == \"f9\" and (love.keyboard.isDown(\"lshift\") or love.keyboard.isDown(\"rshift\")) then
  elseif(\"f9\" == key) then
  elseif key == 'd' and ctrl and love.keyboard.isDown(\"lalt\", \"ralt\") then
  elseif key == \"x\" and not ctrl then
  end
  if other == \"f9\" then end
end
";
        let ix = index(src);
        let k: Vec<(&str, Vec<&str>, usize)> =
            ix.keys.iter().map(|k| (k.key.as_str(), k.modifiers.clone(), k.line)).collect();
        assert_eq!(
            k,
            vec![
                ("f9", vec!["shift"], 3),
                ("f9", vec![], 4),
                ("d", vec!["alt", "ctrl"], 5),
                ("x", vec![], 6),
            ]
        );
    }

    #[test]
    fn wrappers_and_whether_they_call_their_copy() {
        let src = "\
local _orig = loadModSettings
function loadModSettings()
  if _orig then _orig() end
end
local _s = saveModSettings
saveModSettings = function() write() end
local _d = draw
function other() end
function init()
  local prev = Button.click
  Button.click = function(...) return pcall(prev, ...) end
end
";
        let ix = index(src);
        let w: Vec<(&str, &str, bool)> = ix
            .wrappers
            .iter()
            .map(|w| (ix.captures[w.capture].local.as_str(), ix.defs[w.def].name.as_str(), w.calls))
            .collect();
        assert_eq!(
            w,
            vec![("_orig", "loadModSettings", true), ("_s", "saveModSettings", false), ("prev", "Button.click", true)]
        );
    }

    #[test]
    fn entity_fields_calls_buttons_and_pages() {
        let src = "\
local M = {}
M.version = 1
function M.init() end
function update(dude)
  dude.pathQueue = nil
  M.cache = calc(dude)
  dude:say \"hi\"
  buttons[#buttons+1] = Button.new(10, 20, 200, 30, \"menu_mods\", \"active\", true, over)
  Button.new(1, 2, 3, 4, label, \"active\")
  if (menuPage == 85) then menuPage = 86 end
  if 7 == menuPage then end
end
";
        let ix = index(src);
        let w: Vec<(&str, bool, bool, bool)> =
            ix.field_writes.iter().map(|f| (f.name.as_str(), f.local, f.method, f.own_table)).collect();
        assert_eq!(
            w,
            vec![
                ("M.version", true, false, true),
                ("M.init", true, true, true),
                ("dude.pathQueue", true, false, false),
                ("M.cache", true, false, true),
            ]
        );
        let calls: Vec<(&str, usize)> = ix.calls.iter().map(|c| (c.name.as_str(), c.line)).collect();
        assert_eq!(calls, vec![("calc", 6), ("dude:say", 7), ("Button.new", 8), ("Button.new", 9)]);
        let b: Vec<(&str, usize)> = ix.buttons.iter().map(|b| (b.label.as_str(), b.line)).collect();
        assert_eq!(b, vec![("menu_mods", 8)]);
        let p: Vec<(&str, usize)> = ix.pages.iter().map(|p| (p.page.as_str(), p.line)).collect();
        assert_eq!(p, vec![("85", 10), ("86", 10), ("7", 11)]);
    }

    #[test]
    fn names_resolve_to_their_declaration() {
        let src = "\
local count = 0
function infantryUpdate(dt)
  local i = 1
  for j = 1, 3 do
    tick(count, i, j, dt)
  end
  local t = { count = 1, [i] = 2 }
  goto done
  ::done::
end
";
        let (ix, uses) = index_watching(src, &[(5, 5), (7, 8)]);
        assert_eq!(ix.function_path(ix.lines[4].function), "infantryUpdate");
        let free = free_names(&uses, 5, 5);
        let got: Vec<(&str, &Binding)> = free.iter().map(|u| (u.name.as_str(), &u.binding)).collect();
        let local = |function: &str, decl: &str, line: usize| Binding::Local {
            function: function.to_string(),
            decl: decl.to_string(),
            line,
        };
        assert_eq!(
            got,
            vec![
                ("tick", &Binding::Global),
                ("count", &local("<main>", "local count = 0", 1)),
                ("i", &local("infantryUpdate", "local i = 1", 3)),
                ("j", &local("infantryUpdate", "for j = 1, 3 do", 4)),
                ("dt", &local("infantryUpdate", "function infantryUpdate(dt)", 2)),
            ]
        );
        // A table key and a label are not names; `t` is declared on the watched line.
        let names: Vec<&str> = free_names(&uses, 7, 8).iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, vec!["i"]);
        assert!(local("f", "local x", 1).same(&local("f", "local x", 9)));
        assert!(!local("f", "local x", 1).same(&Binding::Global));
    }

    #[test]
    fn block_deltas_and_conditions() {
        assert_eq!(block_delta("if(#smartBatch > 1) then\n  a()\nelse"), 1);
        assert_eq!(block_delta("  end\nend"), -2);
        assert_eq!(block_delta("for i = 1, 2 do x() end while y do end repeat until z"), 0);
        assert_eq!(block_delta("t.end_ = 1 -- end\nlocal s = \"if\""), 0);
        assert_eq!(block_delta("f = function() end"), 0);
        let c = conditions("if(unit.faction ~= target.faction) then\nelseif x then\nwhile (a) and (b) do end");
        assert_eq!(
            c,
            vec![
                vec!["unit", ".", "faction", "~=", "target", ".", "faction"],
                vec!["x"],
                vec!["(", "a", ")", "and", "(", "b", ")"],
            ]
        );
    }

    #[test]
    fn never_panics_on_broken_text() {
        for src in [
            "end end until", "function", "local", "if (", "x = ", "require", "local x =", "if then",
            "a.b, = 1", "if key == then", "if \"x\" == key", "pcall(", "io.open(", "love.filesystem.write(",
            "local c = love.keyboard.isDown(", "local o = f\nf = function() o(", "Button.new(1,2,3,4,",
            "Button.new(", "menuPage ==", "== menuPage", "if (", "while x", "::",
        ] {
            let ix = index(src);
            assert_eq!(ix.lines.len(), src.lines().count());
            let (_, uses) = index_watching(src, &[(1, 3)]);
            let _ = free_names(&uses, 1, 3);
            let _ = (block_delta(src), conditions(src));
        }
    }
}
