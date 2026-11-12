//! Compile check for patched Lua files, using the game's own `lua51.dll` (64-bit LuaJIT).
//!
//! The simulation compiles each patched file the way the game will: `luaL_loadbuffer` on a fresh
//! state, which parses and compiles without running anything. The simulation patches the text
//! the DLL prepares (`prepare::prepare_buffer`), so what is compiled here is what the game
//! loads. Nothing here changes what the game runs. `run_chunk` runs a chunk, for tests of
//! QuarterMaster's own Lua (the main.lua footer).
//!
//! Launcher only: nothing on the DLL path calls this module, so it keeps the crate's default
//! lints and carries no deny attributes.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::path::{Path, PathBuf};

use crate::patch::module::ModulePatch;
use crate::patch::origin::{FileTrace, JournalEdit};

type NewState = unsafe extern "C" fn() -> *mut c_void;
type Close = unsafe extern "C" fn(*mut c_void);
type LoadBuffer = unsafe extern "C" fn(*mut c_void, *const c_char, usize, *const c_char) -> c_int;
type ToLString = unsafe extern "C" fn(*mut c_void, c_int, *mut usize) -> *const c_char;
type PCall = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int) -> c_int;
type OpenLibs = unsafe extern "C" fn(*mut c_void);
type GetTop = unsafe extern "C" fn(*mut c_void) -> c_int;

/// `lua_pcall`'s "all results".
const LUA_MULTRET: c_int = -1;

/// A compile error, with its line in the text that was compiled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub line: Option<usize>,
    /// LuaJIT's message without the `name:line:` prefix.
    pub message: String,
    /// The line named by "(to close 'x' at line N)", when the message has one.
    pub opened_at: Option<usize>,
}

pub struct LuaCompiler {
    dll: PathBuf,
    new_state: NewState,
    close: Close,
    load_buffer: LoadBuffer,
    to_lstring: ToLString,
    pcall: PCall,
    open_libs: OpenLibs,
    get_top: GetTop,
    // Keeps the library loaded for as long as the function pointers above are used.
    _lib: libloading::Library,
}

// The function pointers are plain C functions and every call uses its own fresh state.
unsafe impl Send for LuaCompiler {}
unsafe impl Sync for LuaCompiler {}

impl std::fmt::Debug for LuaCompiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuaCompiler").field("dll", &self.dll).finish()
    }
}

impl LuaCompiler {
    /// `lua51.dll` in the game folder, when it is there.
    pub fn find(game_dir: &Path) -> Option<PathBuf> {
        let p = game_dir.join("lua51.dll");
        p.is_file().then_some(p)
    }

    /// Loads the game's `lua51.dll`. Its own dependencies are looked up in System32 only, so
    /// nothing else from the game folder (such as QuarterMaster's `winmm.dll`) is loaded.
    pub fn load(dll: &Path) -> Result<Self, String> {
        let lib = load_library(dll).map_err(|e| format!("could not load {}: {e}", dll.display()))?;
        unsafe {
            let sym = |name: &[u8]| -> Result<*const c_void, String> {
                lib.get::<*const c_void>(name)
                    .map(|s| *s)
                    .map_err(|e| format!("{} has no {}: {e}", dll.display(), String::from_utf8_lossy(name)))
            };
            let new_state: NewState = std::mem::transmute(sym(b"luaL_newstate\0")?);
            let close: Close = std::mem::transmute(sym(b"lua_close\0")?);
            let load_buffer: LoadBuffer = std::mem::transmute(sym(b"luaL_loadbuffer\0")?);
            let to_lstring: ToLString = std::mem::transmute(sym(b"lua_tolstring\0")?);
            let pcall: PCall = std::mem::transmute(sym(b"lua_pcall\0")?);
            let open_libs: OpenLibs = std::mem::transmute(sym(b"luaL_openlibs\0")?);
            let get_top: GetTop = std::mem::transmute(sym(b"lua_gettop\0")?);
            Ok(Self {
                dll: dll.to_path_buf(),
                new_state,
                close,
                load_buffer,
                to_lstring,
                pcall,
                open_libs,
                get_top,
                _lib: lib,
            })
        }
    }

    pub fn dll(&self) -> &Path {
        &self.dll
    }

    /// Compiles `text` as chunk `name`. Nothing is run.
    pub fn compile(&self, name: &str, text: &str) -> Result<(), CompileError> {
        let chunk = format!("={name}\0");
        unsafe {
            let state = (self.new_state)();
            if state.is_null() {
                return Err(CompileError {
                    line: None,
                    message: "the Lua runtime could not start (out of memory)".into(),
                    opened_at: None,
                });
            }
            let rc = (self.load_buffer)(state, text.as_ptr().cast(), text.len(), chunk.as_ptr().cast());
            let result = if rc == 0 {
                Ok(())
            } else {
                let mut len = 0usize;
                let ptr = (self.to_lstring)(state, -1, &mut len);
                let raw = if ptr.is_null() {
                    format!("error {rc}")
                } else {
                    CStr::from_ptr(ptr).to_string_lossy().into_owned()
                };
                Err(parse_error(name, &raw))
            };
            (self.close)(state);
            result
        }
    }
}

impl LuaCompiler {
    /// Runs `text` as chunk `name` in a fresh state with the standard libraries. Returns the
    /// chunk's first result when it is a string or a number, or the compile or runtime error.
    /// For tests of QuarterMaster's own Lua; never used with mod code.
    pub fn run_chunk(&self, name: &str, text: &str) -> Result<Option<String>, String> {
        let chunk = format!("={name}\0");
        unsafe {
            let state = (self.new_state)();
            if state.is_null() {
                return Err("the Lua runtime could not start (out of memory)".into());
            }
            (self.open_libs)(state);
            let mut rc = (self.load_buffer)(state, text.as_ptr().cast(), text.len(), chunk.as_ptr().cast());
            if rc == 0 {
                rc = (self.pcall)(state, 0, LUA_MULTRET, 0);
            }
            // On success the results start at 1; on error the message is on top.
            let first = if (self.get_top)(state) >= 1 {
                self.string_at(state, if rc == 0 { 1 } else { -1 })
            } else {
                None
            };
            (self.close)(state);
            if rc == 0 {
                Ok(first)
            } else {
                Err(first.unwrap_or_else(|| format!("error {rc}")))
            }
        }
    }

    /// The value at stack `index` as a string, when it is a string or a number.
    unsafe fn string_at(&self, state: *mut c_void, index: c_int) -> Option<String> {
        let mut len = 0usize;
        let ptr = (self.to_lstring)(state, index, &mut len);
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr.cast::<u8>(), len);
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

#[cfg(windows)]
fn load_library(dll: &Path) -> Result<libloading::Library, libloading::Error> {
    use libloading::os::windows::{Library, LOAD_LIBRARY_SEARCH_SYSTEM32};
    unsafe { Library::load_with_flags(dll, LOAD_LIBRARY_SEARCH_SYSTEM32).map(Into::into) }
}

#[cfg(not(windows))]
fn load_library(dll: &Path) -> Result<libloading::Library, libloading::Error> {
    unsafe { libloading::Library::new(dll) }
}

/// LuaJIT's `LUA_IDSIZE`: a `=name` chunk shows at most this many bytes, less one.
const LUA_IDSIZE: usize = 60;

/// Splits `name:12: msg` into line 12 and `msg`, and finds "at line N" in the message.
/// LuaJIT cuts a long chunk name to `LUA_IDSIZE - 1` bytes in its messages.
fn parse_error(name: &str, raw: &str) -> CompileError {
    let shown = name.as_bytes().get(..LUA_IDSIZE - 1).unwrap_or(name.as_bytes());
    let shown = String::from_utf8_lossy(shown);
    let rest = raw.strip_prefix(shown.as_ref()).and_then(|r| r.strip_prefix(':'));
    let (line, message) = match rest.and_then(|r| r.split_once(':')) {
        Some((n, msg)) if n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty() => {
            (n.parse().ok(), msg.trim().to_string())
        }
        _ => (None, raw.trim().to_string()),
    };
    let opened_at = message.find("at line ").and_then(|i| {
        let digits: String = message[i + 8..].chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    });
    CompileError { line, message, opened_at }
}

/// A compile error in a patched file, mapped back to the file and its patches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCompileError {
    /// 1-based line in the patched file; `None` when the error is in QuarterMaster's own
    /// wrapper (usually: the file ends inside an unclosed block).
    pub line: Option<usize>,
    pub opened_at: Option<usize>,
    pub message: String,
    /// The first patch (record index) after which the file stops compiling.
    pub breaking_record: Option<usize>,
    /// The patch that wrote the error line, when a patch did.
    pub line_record: Option<usize>,
}

/// Compiles the traced file as the game would. `original` is the prepared text the trace
/// starts from and `trace.text` is what the game loads. `to_file` maps a 1-based line of that
/// text to the file's line, `None` for QuarterMaster's own lines. `Ok(None)` when it compiles;
/// `Err` when even the game's own file does not (nothing to report about patches then).
pub fn check_file(
    compiler: &LuaCompiler,
    target: &str,
    original: &str,
    trace: &FileTrace,
    to_file: &dyn Fn(usize) -> Option<usize>,
) -> Result<Option<FileCompileError>, CompileError> {
    let Err(err) = compiler.compile(target, &trace.text) else {
        return Ok(None);
    };
    compiler.compile(target, original)?;
    let total = trace.lines.len();
    let shown = |l: usize| Some(l).filter(|&n| n >= 1 && n <= total && to_file(n).is_some());
    let (game_line, game_opened) = (err.line.and_then(shown), err.opened_at.and_then(shown));
    let line_record = game_line
        .or(game_opened)
        .and_then(|n| trace.lines.get(n - 1))
        .and_then(|o| o.edit())
        .and_then(|e| trace.edits.get(e))
        .map(|e| e.record);
    Ok(Some(FileCompileError {
        line: game_line.and_then(to_file),
        opened_at: game_opened.and_then(to_file),
        message: err.message,
        breaking_record: breaking_record(compiler, target, original, &trace.edits),
        line_record,
    }))
}

/// Compiles a mod module as the game loads it: its own text, unpatched, under its chunk name
/// `=[qm <name> "<source>"]` (D44, §6.2). The error line is the module file's own line.
pub fn check_module(compiler: &LuaCompiler, module: &ModulePatch) -> Result<(), CompileError> {
    let chunk = module.chunk_name();
    let name = chunk.strip_prefix('=').unwrap_or(&chunk);
    compiler.compile(name, &module.content)
}

/// Replays the journal one patch at a time and returns the first patch after which the file
/// no longer compiles.
fn breaking_record(compiler: &LuaCompiler, target: &str, original: &str, edits: &[JournalEdit]) -> Option<usize> {
    let mut text = original.to_string();
    let mut i = 0;
    while i < edits.len() {
        let record = edits[i].record;
        while i < edits.len() && edits[i].record == record {
            let e = &edits[i];
            let start = e.start.min(text.len());
            let end = (start + e.removed).min(text.len());
            text.replace_range(start..end, &e.inserted);
            i += 1;
        }
        if compiler.compile(target, &text).is_err() {
            return Some(record);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handshake::LaunchConfig;
    use crate::patch::origin::trace;
    use crate::prepare::{prepare_buffer, FileView};

    fn compiler() -> Option<LuaCompiler> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        [root.join("../../../game"), root.join("../../game"), PathBuf::from("D:/WindowsGames/SteamLibrary/steamapps/common/MudandBlood")]
            .iter()
            .find_map(|d| LuaCompiler::find(d))
            .and_then(|p| LuaCompiler::load(&p).ok())
    }

    #[test]
    fn parses_luajit_messages() {
        let e = parse_error("main.lua", "main.lua:12: 'end' expected (to close 'function' at line 3) near '<eof>'");
        assert_eq!(e.line, Some(12));
        assert_eq!(e.opened_at, Some(3));
        assert!(e.message.starts_with("'end' expected"));
        let e = parse_error("main.lua", "weird");
        assert_eq!(e.line, None);
    }

    #[test]
    fn parses_a_long_chunk_name_as_luajit_cuts_it() {
        let name = format!("[qm {} \"lib/x.lua\"]", "a".repeat(70));
        let raw = format!("{}:4: unexpected symbol near 'x'", &name[..59]);
        let e = parse_error(&name, &raw);
        assert_eq!(e.line, Some(4));
        assert_eq!(e.message, "unexpected symbol near 'x'");
    }

    fn module(name: &str, source: &str, content: &str) -> ModulePatch {
        let mut m: ModulePatch = serde_json::from_value(serde_json::json!({
            "source": source, "name": name
        }))
        .unwrap();
        m.display_source = source.to_string();
        m.content = content.to_string();
        m
    }

    #[test]
    fn compiles_a_module_under_its_chunk_name() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let ok = module("lib/ok", "ok.lua", "local M = {}\nreturn M\n");
        assert_eq!(check_module(&c, &ok), Ok(()));
        let bad = module("lib/bad", "bad.lua", "local M = {}\nfunction M.f(\nreturn M\n");
        let e = check_module(&c, &bad).unwrap_err();
        assert_eq!(e.line, Some(3), "{e:?}");
        // A long name is cut in LuaJIT's message; the line is still found.
        let long = module(&"n".repeat(80), "src/long.lua", "x = = 1\n");
        let e = check_module(&c, &long).unwrap_err();
        assert_eq!(e.line, Some(1), "{e:?}");
        assert!(e.message.contains("unexpected symbol"), "{e:?}");
    }

    #[test]
    fn compiles_with_the_games_luajit() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        assert!(c.compile("ok.lua", "local x = 1\nreturn x\n").is_ok());
        let e = c.compile("bad.lua", "local x = 1\nif x then\n").unwrap_err();
        assert_eq!(e.line, Some(3));
        assert_eq!(e.opened_at, Some(2));
    }

    #[test]
    fn finds_the_breaking_patch_and_maps_main_lua_lines() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let src = "function a()\n  return 1\nend\n";
        // The simulation compiles the prepared main.lua: QuarterMaster's prefix comes first.
        let prepared = prepare_buffer("main.lua", src, &LaunchConfig::default());
        let at = prepared.text.find(src).expect("the vanilla part");
        let edit = |record: usize, start: usize, removed: usize, inserted: &str| JournalEdit {
            patch_id: format!("p{record}"),
            patch_name: format!("p{record}"),
            mod_id: format!("m{record}"),
            record,
            start: at + start,
            removed,
            inserted: inserted.into(),
            site: 0,
            block: crate::patch::linemap::NO_BLOCK,
        };
        // Record 0 adds a harmless line; record 1 opens an `if` that is never closed.
        let t = trace(&prepared.text, vec![edit(0, 12, 0, "  local y = 2\n"), edit(1, 26, 0, "  if y then\n")]);
        let view = FileView::new(&t, &prepared);
        let to_file = |n: usize| view.line(n);
        let err = check_file(&c, "main.lua", &prepared.text, &t, &to_file).unwrap().expect("does not compile");
        assert_eq!(err.breaking_record, Some(1));
        assert_eq!(err.opened_at, Some(1), "file numbering, not the prepared text's");
        let ok = trace(&prepared.text, vec![edit(0, 12, 0, "  local y = 2\n")]);
        let view = FileView::new(&ok, &prepared);
        let to_file = |n: usize| view.line(n);
        assert_eq!(check_file(&c, "main.lua", &prepared.text, &ok, &to_file).unwrap(), None);
    }

    #[test]
    fn the_prepared_main_lua_with_the_footer_compiles() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let src = "local x = 1\nfunction love.load()\nend\nfunction love.quit()\nend\n";
        let prepared = prepare_buffer("main.lua", src, &LaunchConfig::default());
        assert!(prepared.text.ends_with(crate::QM_RUNTIME_FOOTER_PAYLOAD));
        assert_eq!(c.compile("main.lua", &prepared.text), Ok(()));
    }

    /// The game's side of a footer test: a stub `qm` module that counts its calls, and a
    /// `love` table whose handler, love.quit and love.load are the game's. `load_body` runs
    /// inside love.load. The footer comes after it, then `checks`.
    const FOOTER_PRELUDE: &str = r#"
local calls = { clean = 0, crash = 0, mod_quit = 0, game_quit = 0, mod_err = 0 }
local qm_stub = {
  record_lua_crash = function(msg, trace)
    calls.crash = calls.crash + 1
    return "patch X"
  end,
  record_clean_exit = function()
    calls.clean = calls.clean + 1
  end,
}
package.preload.qm = function() return qm_stub end
local handled = {}
love = {}
love.errorhandler = function(msg)
  handled[#handled + 1] = "game: " .. msg
  return "game handler"
end
local quit_result, quit_fails = nil, false
love.quit = function()
  calls.game_quit = calls.game_quit + 1
  if quit_fails then
    error("quit failed")
  end
  return quit_result
end
love.load = function(a, b)
  LOAD_BODY
  return a + b, "second"
end
local game_handler, game_quit, game_load = love.errorhandler, love.quit, love.load
"#;

    /// Runs the footer against the stubs; `Ok` when every check passed.
    fn run_footer(c: &LuaCompiler, setup: &str, load_body: &str, checks: &str) -> Result<Option<String>, String> {
        let text = format!(
            "{}{setup}\n{}\n{checks}\nreturn \"ok\"\n",
            FOOTER_PRELUDE.replace("LOAD_BODY", load_body),
            crate::QM_RUNTIME_FOOTER_PAYLOAD
        );
        c.run_chunk("footer_test", &text)
    }

    #[test]
    fn the_footer_reports_the_asset_winners_after_love_load() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let setup = r#"
local reported = {}
qm_stub.record_asset_winners = function(text) reported[#reported + 1] = text end
local winner_text = "sounds/garand1\taxissw"
__qm_asset_winner_text = function() return winner_text end
"#;
        let checks = r#"
assert(#reported == 0, "nothing before love.load")
love.load(1, 1)
assert(#reported == 1 and reported[1] == "sounds/garand1\taxissw", tostring(reported[1]))
-- No asset was replaced: nothing is written.
winner_text = ""
love.load(1, 1)
assert(#reported == 1, "an empty list is not reported")
-- An older DLL has no such function: love.load still returns.
qm_stub.record_asset_winners = nil
winner_text = "x\ty"
local sum = love.load(2, 2)
assert(sum == 4)
"#;
        assert_eq!(run_footer(&c, setup, "", checks), Ok(Some("ok".into())));
        // Without the prelude's function the footer does nothing here.
        let checks = "love.load(1, 1)\n";
        assert_eq!(run_footer(&c, "", "", checks), Ok(Some("ok".into())));
    }

    /// The game's side of the debug and developer keys: a stub keyboard, mouse and player.
    const DEV_SETUP: &str = r#"
local dumped = {}
qm_stub.dump_game_files = function(dir, filter)
  dumped[#dumped + 1] = "game:" .. tostring(filter)
  return true, 3, "x"
end
qm_stub.dump_patched = function()
  dumped[#dumped + 1] = "patched"
  return true, 2, "x"
end
local held = {}
love.keyboard = { isDown = function(k) return held[k] == true end }
local game_keys = {}
love.keypressed = function(key) game_keys[#game_keys + 1] = key end
local game_keypressed = love.keypressed
local strikes = {}
p38Strike = function(x, y, n) strikes[#strikes + 1] = x .. "," .. y .. "," .. n end
local releases = 0
love.mousereleased = function() releases = releases + 1 end
player = { textCue = {}, screenx = 100, screeny = 0, gameState = 0 }
showAI, devBlast, QM, GDumper, GConsole = 0, 0, nil, nil, nil
"#;

    #[test]
    fn the_footer_adds_the_debug_and_developer_keys_only_when_asked() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        // Neither mode: the game's handlers are left alone.
        let checks = "assert(love.keypressed == game_keypressed)\nlove.mousereleased(1, 1, 2)\nassert(releases == 1)\n";
        assert_eq!(run_footer(&c, DEV_SETUP, "", checks), Ok(Some("ok".into())));

        let setup = format!("{DEV_SETUP}qm_stub.debug, qm_stub.dev = true, true\n");
        let checks = r#"
love.keypressed("f1")
assert(showAI == 1)
love.keypressed("f1")
assert(showAI == 0)
assert(#game_keys == 0, "the developer keys do not reach the game")
love.keypressed("h")
assert(game_keys[1] == "h", "other keys do")
love.mousereleased(10, 20, 2)
assert(#strikes == 0 and releases == 1, "no strike while the dev blast is off")
love.keypressed("f5")
assert(devBlast == 1)
love.mousereleased(10, 20, 2)
assert(strikes[1] == "110,20,6", tostring(strikes[1]))
love.mousereleased(10, 20, 1)
assert(#strikes == 1, "only the right button")
held.lctrl, held.lshift = true, true
love.keypressed("d")
assert(dumped[1] == "game:nil", tostring(dumped[1]))
love.keypressed("p")
assert(dumped[2] == "patched")
held.lalt = true
love.keypressed("d")
assert(dumped[3] == "game:*", tostring(dumped[3]))
assert(#player.textCue == 6, #player.textCue)
"#;
        assert_eq!(run_footer(&c, &setup, "", checks), Ok(Some("ok".into())));

        // Developer commands alone: no dump keys.
        let setup = format!("{DEV_SETUP}qm_stub.dev = true\nheld.lctrl, held.lshift = true, true\n");
        let checks = "love.keypressed(\"d\")\nassert(#dumped == 0 and game_keys[1] == \"d\")\n";
        assert_eq!(run_footer(&c, &setup, "", checks), Ok(Some("ok".into())));

        // With the input framework the keys are registered there, and the blast hooks the
        // handler the framework calls.
        let setup = format!(
            "{DEV_SETUP}qm_stub.debug, qm_stub.dev = true, true\nlocal registered = {{}}\nQM = {{ Input = {{\n  registerAction = function(def) registered[#registered + 1] = def end,\n  origMouseReleased = function() releases = releases + 1 end,\n}} }}\n"
        );
        let checks = r#"
assert(love.keypressed == game_keypressed, "the framework owns the keys")
assert(#registered == 5, #registered)
assert(registered[1].tab == "quartermaster" and registered[1].default == "d")
assert(registered[1].modifiers[1] == "ctrl" and registered[1].modifiers[2] == "shift")
registered[4].onTrigger()
assert(showAI == 1)
registered[5].onTrigger()
QM.Input.origMouseReleased(1, 2, 2)
assert(strikes[1] == "101,2,6" and releases == 1, tostring(strikes[1]))
"#;
        assert_eq!(run_footer(&c, &setup, "", checks), Ok(Some("ok".into())));
    }

    /// The save stamp (Phase E D85): a save that loads gets `_qm_meta` from `qm.save_info`,
    /// an absent mod keeps the fields an earlier stamp listed while the save has them, and
    /// the game's serializer and SaveGuard's fork both write the stamp so the launcher reads
    /// it back. Without a stamp in the launch config nothing is set.
    #[test]
    fn a_loaded_save_is_stamped_and_the_serializers_keep_the_stamp() {
        use crate::handshake::{LaunchConfig, SaveStamp};
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let cfg = LaunchConfig {
            save_stamp: Some(
                SaveStamp {
                    profile_id: "p1".into(),
                    profile_name: "My \"best\" run".into(),
                    mods: [("moda".to_string(), "1.0.0".to_string())].into(),
                    fields: [("moda".to_string(), vec!["option_a".to_string()])].into(),
                }
                .into(),
            ),
            ..LaunchConfig::default()
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let sers: Vec<String> = ["decompile/includes/ser.lua", "MnB-Mods/SaveGuard/ser.lua"]
            .iter()
            .filter_map(|rel| std::fs::read_to_string(root.join(rel)).ok())
            .collect();
        if sers.len() < 2 {
            eprintln!("a serializer file is missing; checked {} of 2", sers.len());
        }
        let ser_list: String = sers
            .iter()
            .map(|s| format!("loadstring([=====[{s}]=====])(),\n"))
            .collect();
        let game = |info: &str| {
            format!(
                r#"
package.loaded.qm = {{ save_info = [=====[{info}]=====] }}
{prelude}
updateProfile = function(p) p.loaded = true end
player = {{ experience = 1, option_a = 5, gone_field = 2,
  _qm_meta = {{ profile_id = "old", fields = {{ gone = {{ "gone_field", "vanished" }}, moda = {{ "stale" }} }} }} }}
updateProfile(player)
assert(player.loaded, "the game's own updateProfile ran")
local out = {{}}
local m = player._qm_meta
if STAMPED then
  assert(m.profile_id == "p1" and m.profile_name == 'My "best" run', tostring(m.profile_name))
  assert(m.mods.moda == "1.0.0")
  assert(#m.fields.moda == 1 and m.fields.moda[1] == "option_a")
  assert(#m.fields.gone == 1 and m.fields.gone[1] == "gone_field")
  assert(type(m.timestamp) == "string" and #m.timestamp == 20, tostring(m.timestamp))
  -- The stamp is a copy: a second load does not grow it.
  updateProfile(player)
  assert(#player._qm_meta.fields.gone == 1)
  for _, ser in ipairs({{ {ser_list} }}) do
    local text = ser(player)
    local back = loadstring(text)()
    assert(back._qm_meta.fields.moda[1] == "option_a", "the serializer kept the stamp")
    out[#out + 1] = text
  end
else
  assert(m.profile_id == "old" and m.timestamp == nil, "no stamp, nothing changes")
end
return table.concat(out, "\n--NEXT--\n")
"#,
                prelude = crate::patch::loader::STEAM_SAFETY_PAYLOAD
            )
        };
        let stamped = format!("STAMPED = true\n{}", game(&cfg.save_info_lua(&|_| true)));
        let text = c.run_chunk("save_stamp", &stamped).unwrap().unwrap_or_default();
        let saves: Vec<&str> = text.split("\n--NEXT--\n").filter(|s| !s.is_empty()).collect();
        assert_eq!(saves.len(), sers.len());
        for save in saves {
            let s = qm_schema::SaveSummary::from_ser_str("saveMNB3.lua", save).unwrap();
            let meta = s.qm_meta.unwrap();
            assert_eq!(meta.profile_id.as_deref(), Some("p1"));
            assert_eq!(meta.profile_name.as_deref(), Some("My \"best\" run"));
            assert_eq!(meta.mods["moda"], "1.0.0");
            assert_eq!(meta.fields["moda"], ["option_a"]);
            assert_eq!(meta.fields["gone"], ["gone_field"]);
            assert!(meta.timestamp.is_some());
        }
        let plain = LaunchConfig::default().save_info_lua(&|_| true);
        assert_eq!(plain, "");
        assert_eq!(c.run_chunk("save_stamp_none", &game(&plain)), Ok(Some(String::new())));
    }

    /// Phase E D88: QuarterMaster's wrappers are put back at the end of love.load. A mod that
    /// replaced a wrapped function gets QuarterMaster's part around its version; a mod that
    /// wrapped the wrapper makes it run once, not twice; the save check runs once after a
    /// mod redefined `updateProfile`.
    #[test]
    fn wrappers_are_put_back_after_love_load_and_run_once() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let chunk = format!(
            r#"
package.loaded.qm = {{}}
love = {{ audio = {{}}, graphics = {{}}, filesystem = {{}} }}
{prelude}
local log = {{}}
local function note(s) log[#log + 1] = s end
-- The game's functions, defined after the startup code.
updateProfile = function(p) note("game:updateProfile") end
weaponCreator = function(w, tag) note("game:weaponCreator") end
specClick = function() note("game:specClick") end
firstNamer = function() return "game name" end
local checks = 0
local real_check = __qm_faction_save_cleanup
__qm_faction_save_cleanup = function(p) checks = checks + 1 return real_check(p) end
-- What a faction does in love.load.
local function faction()
  __qm_wrap("weaponCreator", function(orig) return function(w, tag)
    note("qm:weaponCreator")
    return orig(w, tag)
  end end)
  __qm_wrap("specClick", function(orig) return function(...)
    note("qm:specClick")
    return orig(...)
  end end)
  __qm_wrap("firstNamer", function() return function() return "faction name" end end)
end
love.load = function()
  faction()
  -- A mod replaces weaponCreator and does not call the one it found.
  weaponCreator = function(w, tag) note("mod:weaponCreator") end
  -- A mod wraps specClick and calls the one it found.
  local seen = specClick
  specClick = function(...) note("mod:specClick") return seen(...) end
  -- A mod redefines updateProfile and the soldier names.
  updateProfile = function(p) note("mod:updateProfile") end
  firstNamer = function() return "mod name" end
end
{footer}
love.load()
log = {{}}
weaponCreator({{}}, "k98")
assert(table.concat(log, ",") == "qm:weaponCreator,mod:weaponCreator", table.concat(log, ","))
log = {{}}
specClick()
assert(table.concat(log, ",") == "qm:specClick,mod:specClick,game:specClick", table.concat(log, ","))
log = {{}}
updateProfile({{}})
assert(table.concat(log, ",") == "mod:updateProfile", table.concat(log, ","))
assert(checks == 1, "the save check ran " .. checks .. " times")
assert(firstNamer() == "faction name")
-- Nothing was replaced since: a second pass changes nothing.
assert(#__qm_reassert_wrappers() == 0)
log = {{}}
specClick()
assert(#log == 3)
return "ok"
"#,
            prelude = crate::patch::loader::STEAM_SAFETY_PAYLOAD,
            footer = crate::QM_RUNTIME_FOOTER_PAYLOAD
        );
        assert_eq!(c.run_chunk("reassert", &chunk), Ok(Some("ok".into())));
    }

    /// Phase E D89, D90: in a diagnostic test run the startup code notes each change of a
    /// watched global with the line that made it, the footer counts who calls the watched
    /// functions, and both are reported after love.load, after the first frame and at quit.
    /// A normal launch has no `qm.diagnostics` and sets nothing up.
    #[test]
    fn a_diagnostic_run_traces_globals_and_counts_hook_callers() {
        use crate::handshake::{Diagnostics, LaunchConfig};
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let cfg = LaunchConfig {
            diagnostics: Some(
                Diagnostics {
                    globals: vec!["loadModSettings".into(), "counter".into()],
                    hooks: vec!["onDeath".into(), "Events.fire".into(), "missing".into()],
                }
                .into(),
            ),
            ..LaunchConfig::default()
        };
        assert_eq!(LaunchConfig::default().diagnostics_lua(), "");
        let game = |diag: &str| {
            format!(
                r#"
local reported = {{}}
package.loaded.qm = {{ diagnostics = [=====[{diag}]=====], record_diagnostics = function(t) reported[#reported + 1] = t end }}
love = {{ audio = {{}}, graphics = {{}}, filesystem = {{}} }}
{prelude}
local here = function() return debug.getinfo(2, "l").currentline end
local l1 = here() + 1
loadModSettings = function() end
local l2 = here() + 1
loadModSettings = function() end
counter = 5
onDeath = function() return "dead" end
Events = {{ fire = function() end }}
local l3
love.load = function()
  l3 = here() + 1
  counter = 6
end
love.update = function() end
{footer}
love.load()
if not TRACED then
  assert(__qm_diag == nil and #reported == 0, "a normal launch sets nothing up")
  return "plain"
end
assert(#reported == 1, "reported after love.load")
local lc = here() + 1
for _ = 1, 2 do assert(onDeath() == "dead") end
onDeath()
Events.fire()
love.update()
assert(#reported == 2, "reported after the first frame")
assert(debug.gethook() == nil, "the line hook is off after the first frame")
love.quit()
assert(#reported == 3, "reported at quit")
local text = "\n" .. reported[3] .. "\n"
local function has(line) assert(text:find("\n" .. line .. "\n", 1, true), "missing: " .. line .. " in " .. text) end
local function has_like(pat) assert(text:find(pat), "missing: " .. pat .. " in " .. text) end
has_like("\nG\tloadModSettings\tfunction\tfunction: [^\t]+\t=diag\t" .. l1 .. "\n")
has_like("\nG\tloadModSettings\tfunction\tfunction: [^\t]+\t=diag\t" .. l2 .. "\n")
has("G\tcounter\tnumber\t5\t=diag\t" .. (l2 + 1))
has("G\tcounter\tnumber\t6\t=diag\t" .. l3)
has("H\tonDeath\t3")
has("C\tonDeath\t=diag\t" .. lc .. "\t2")
has("C\tonDeath\t=diag\t" .. (lc + 1) .. "\t1")
has("H\tEvents.fire\t1")
has("H\tmissing\t-1")
return "traced"
"#,
                prelude = crate::patch::loader::STEAM_SAFETY_PAYLOAD,
                footer = crate::QM_RUNTIME_FOOTER_PAYLOAD
            )
        };
        let traced = format!("TRACED = true\n{}", game(&cfg.diagnostics_lua()));
        assert_eq!(c.run_chunk("diag", &traced), Ok(Some("traced".into())));
        assert_eq!(c.run_chunk("diag", &game("")), Ok(Some("plain".into())));
    }

    /// Declared save content (Phase E D87): a weapon of a mod that is not loaded becomes the
    /// base weapon its mod.json names (or the K98 when that is no game weapon); a weapon of
    /// a loaded mod, and one no mod declared, stay as they are.
    #[test]
    fn a_weapon_of_a_mod_that_is_off_becomes_its_declared_base() {
        use crate::handshake::{LaunchConfig, SaveContent};
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let content = |pairs: &[(&str, &str)]| {
            SaveContent {
                weapons: pairs.iter().map(|(t, b)| (t.to_string(), b.to_string())).collect(),
            }
            .into()
        };
        let cfg = LaunchConfig {
            save_content: [
                ("gone_mod".to_string(), content(&[("example_rifle", "g43"), ("odd_gun", "not_a_weapon")])),
                ("here_mod".to_string(), content(&[("loaded_rifle", "k98")])),
            ]
            .into(),
            ..LaunchConfig::default()
        };
        let info = cfg.save_info_lua(&|id| id == "here_mod");
        assert!(info.contains("weapons={") && info.contains("loaded_weapons={"), "{info}");
        // With every mod loaded nothing is passed as gone.
        assert!(cfg.save_info_lua(&|_| true).starts_with("return {loaded_weapons={"));
        let chunk = format!(
            r#"
package.loaded.qm = {{ save_info = [=====[{info}]=====] }}
{prelude}
weaponCreator = function(t, tag) t.tag = tag; t.rebuilt = true end
updateProfile = function(p) end
local gun = function(tag) return {{ tag = tag, maxAmmo = 5, magSprite = 1 }} end
player = {{
  unlock = {{ rifle = {{ "k98", "example_rifle", "loaded_rifle", "unknown_rifle", "odd_gun" }} }},
  squad = {{ gun("example_rifle"), gun("loaded_rifle"), gun("unknown_rifle"), gun("odd_gun") }},
}}
player.squad[1].primarySwap = "example_rifle"
player.squad[2].primarySwap = "loaded_rifle"
updateProfile(player)
local s = player.squad
assert(s[1].tag == "g43" and s[1].rebuilt, "a declared weapon of a mod that is off")
assert(s[1].primarySwap == "g43")
assert(s[2].tag == "loaded_rifle" and not s[2].rebuilt and s[2].primarySwap == "loaded_rifle", "a loaded mod's weapon")
assert(s[3].tag == "unknown_rifle" and not s[3].rebuilt, "a weapon no mod declared")
assert(s[4].tag == "k98" and s[4].rebuilt, "a base that is no game weapon falls back to the K98")
assert(table.concat(player.unlock.rifle, ",") == "k98,g43,loaded_rifle,unknown_rifle,k98", table.concat(player.unlock.rifle, ","))
assert(player.qmFactionWeaponBases == nil)
return "ok"
"#,
            prelude = crate::patch::loader::STEAM_SAFETY_PAYLOAD
        );
        assert_eq!(c.run_chunk("save_content", &chunk), Ok(Some("ok".into())));
    }

    /// A save that used a faction: a weapon the running game builds (a loaded weapon mod that
    /// declares nothing) stays; a faction weapon whose faction is off becomes its base, and
    /// any other weapon the game does not know becomes the K98.
    #[test]
    fn a_faction_save_keeps_the_weapons_the_running_game_builds() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let chunk = format!(
            r#"
{prelude}
-- As the game's builder: a tag it does not know becomes a K98. "mod_rifle" is a loaded mod's.
weaponCreator = function(t, tag)
  if tag ~= "mod_rifle" and tag ~= "g43" then tag = "k98" end
  t.tag = tag; t.maxAmmo = 5; t.magSprite = 1; t.rebuilt = true
end
updateProfile = function(p) end
local gun = function(tag) return {{ tag = tag, maxAmmo = 5, magSprite = 1 }} end
player = {{
  qmFactionWeaponBases = {{ faction_gun = "g43" }},
  unlock = {{ rifle = {{ "mod_rifle", "faction_gun", "stray_gun" }} }},
  squad = {{ gun("mod_rifle"), gun("faction_gun"), gun("stray_gun") }},
}}
updateProfile(player)
local s = player.squad
assert(s[1].tag == "mod_rifle" and not s[1].rebuilt, "a loaded mod's weapon stays")
assert(s[2].tag == "g43" and s[2].rebuilt, "a faction weapon becomes its base")
assert(s[3].tag == "k98" and s[3].rebuilt, "a weapon nothing builds becomes the K98")
assert(table.concat(player.unlock.rifle, ",") == "mod_rifle,g43,k98", table.concat(player.unlock.rifle, ","))
return "ok"
"#,
            prelude = crate::patch::loader::STEAM_SAFETY_PAYLOAD
        );
        assert_eq!(c.run_chunk("faction_save_weapons", &chunk), Ok(Some("ok".into())));
    }

    /// Steam that does not start: the player gets a message box and the game closes,
    /// instead of the silent crash at the game's first Steam call. Steam that starts is
    /// left alone.
    #[test]
    fn the_prelude_says_so_when_steam_does_not_start() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let stubs = r#"
love = { audio = {}, graphics = {}, filesystem = {}, window = {} }
love.audio.newSource = function(a) return { from = a } end
love.graphics.newImage = function(a) return { from = a } end
local shown, exited = {}, nil
love.window.showMessageBox = function(title, text, kind) shown[#shown + 1] = kind .. ": " .. text end
os.exit = function(code) exited = code end
local steamUp = false
package.preload.luasteam = function() return { init = function() return steamUp end } end
function __test_state() return shown, exited end
function __test_steam(up) steamUp = up end
"#;
        let game = r#"
local Steam = require "luasteam"
__test_steam(true)
assert(Steam.init() == true)
local shown, exited = __test_state()
assert(#shown == 0 and exited == nil, "a running Steam shows nothing")
__test_steam(false)
Steam.init()
shown, exited = __test_state()
assert(#shown == 1 and shown[1]:find("^error: Steam is not running"), tostring(shown[1]))
assert(exited == 1, "the game closes after the message")
return "ok"
"#;
        let chunk = format!("{stubs}\n{}\n{game}", crate::patch::loader::STEAM_SAFETY_PAYLOAD);
        assert_eq!(c.run_chunk("prelude_steam", &chunk), Ok(Some("ok".into())));
    }

    /// QuarterMaster's startup code against a stub `love`: two mods replace one game sound,
    /// one adds a sound, a mod replaces a game image, and the winner list says whose file
    /// each slot ended up with (Phase E D77).
    #[test]
    fn the_prelude_reports_whose_asset_file_won() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        for m in ["moda", "modb"] {
            let sounds = dir.path().join(m).join("assets/sounds");
            std::fs::create_dir_all(&sounds).unwrap();
            std::fs::write(sounds.join("garand1.wav"), b"RIFF").unwrap();
        }
        std::fs::write(dir.path().join("moda/assets/sounds/extraShot.wav"), b"RIFF").unwrap();
        std::fs::create_dir_all(dir.path().join("modb/assets/sprites")).unwrap();
        std::fs::write(dir.path().join("modb/assets/sprites/allied_idle.png"), b"PNG").unwrap();
        let root = dir.path().to_string_lossy().replace('\\', "/");

        let stubs = r#"
love = { audio = {}, graphics = {}, filesystem = {} }
love.audio.newSource = function(a, mode) return { type = "source", from = a } end
love.graphics.newImage = function(a) return { type = "image", from = a } end
love.filesystem.newFileData = function(bytes, name) return { name = name } end
"#;
        let game = format!(
            r#"
-- The game loads its own files after QuarterMaster's startup code.
garand1 = love.audio.newSource("sounds/garand1.wav", "static")
allied_idle = love.graphics.newImage("sprites/allied_idle.png")
local row = function(rel, cat, stem, ext, mode)
  return '"' .. rel .. '"\t"' .. cat .. '"\t"' .. stem .. '"\t"' .. ext .. '"\t"' .. mode .. '"\n'
end
assert(__qm_asset_winner_text() == "", "nothing replaced yet")
__qm_auto_asset_manifest("{root}/moda",
  row("assets/sounds/garand1.wav", "sounds", "garand1", "wav", "static")
  .. row("assets/sounds/extraShot.wav", "sounds", "extraShot", "wav", "static"), "moda")
assert(__qm_asset_winner_text() == "sounds/extrashot\tmoda\nsounds/garand1\tmoda", __qm_asset_winner_text())
__qm_auto_asset_manifest("{root}/modb",
  row("assets/sounds/garand1.wav", "sounds", "garand1", "wav", "static")
  .. row("assets/sprites/allied_idle.png", "sprites", "allied_idle", "png", ""), "modb")
local text = __qm_asset_winner_text()
assert(text == "sounds/extrashot\tmoda\nsounds/garand1\tmodb\nsprites/allied_idle\tmodb", text)
-- A patch an older launcher built names no mod: it still loads, and reports nothing.
__qm_auto_asset_manifest("{root}/moda", row("assets/sounds/garand1.wav", "sounds", "garand1", "wav", "static"))
assert(__qm_asset_winner_text() == text, "the two-argument form reports no winner")
return "ok"
"#
        );
        let chunk = format!("{stubs}\n{}\n{game}", crate::patch::loader::STEAM_SAFETY_PAYLOAD);
        assert_eq!(c.run_chunk("prelude_assets", &chunk), Ok(Some("ok".into())));
    }

    /// Sprite sizes against a stub `love` whose images are userdata with one shared
    /// metatable, as LÖVE's are: a replacement of another resolution is shown at the
    /// original's size, `[sprites]` rules set sizes, and a reload keeps them.
    #[test]
    fn the_prelude_shows_a_replaced_image_at_the_original_size() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let sprites = dir.path().join("moda/assets/sprites");
        std::fs::create_dir_all(&sprites).unwrap();
        for f in ["top_m1.png", "side_m1.png"] {
            std::fs::write(sprites.join(f), b"PNG").unwrap();
        }
        let root = dir.path().to_string_lossy().replace('\\', "/");

        let mut rules = std::collections::BTreeMap::new();
        let rule = |size, scale, like: Option<&str>| crate::patch::SpriteRule {
            size,
            scale,
            like: like.map(str::to_string),
        };
        rules.insert("top_big".to_string(), rule(Some([150.0, 100.0]), None, None));
        rules.insert("top_half".to_string(), rule(None, Some(0.5), None));
        rules.insert("top_copy".to_string(), rule(None, None, Some("top_big")));
        rules.insert("no_such_image".to_string(), rule(None, Some(2.0), None));
        rules.insert("two keys".to_string(), rule(None, Some(2.0), None));
        rules.insert("both".to_string(), rule(Some([1.0, 1.0]), Some(2.0), None));
        rules.insert("zero".to_string(), rule(None, Some(0.0), None));
        rules.insert("itself".to_string(), rule(None, None, Some("itself")));
        let (table, warnings) = crate::patch::loader::sprite_rules_lua(&rules);
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        let (patch, _) = crate::patch::loader::build_sprite_rules_patch("moda", &rules, 0);
        let patch = patch.expect("four rules are usable");
        assert_eq!(patch.id(), Some("moda_sprite_sizes"));
        assert_eq!(patch.name(), Some("Sprite Sizes (4)"));

        let stubs = r#"
-- Images as LÖVE makes them: userdata sharing one metatable whose __index holds the methods.
local dims = setmetatable({}, { __mode = "k" })
local proto = newproxy(true)
getmetatable(proto).__index = {
  getWidth = function(self) return dims[self][1] end,
  getHeight = function(self) return dims[self][2] end,
  getDimensions = function(self) return dims[self][1], dims[self][2] end,
  typeOf = function(self, t) return t == "Image" end,
}
local game_sizes = { ["sprites/top_m1.png"] = { 100, 40 }, ["sprites/side_m1.png"] = { 60, 30 },
  ["sprites/top_big.png"] = { 300, 300 }, ["sprites/top_half.png"] = { 400, 200 }, ["sprites/top_copy.png"] = { 10, 10 } }
local mod_sizes = { ["top_m1.png"] = { 400, 160 }, ["side_m1.png"] = { 60, 60 } }
love = { audio = {}, graphics = {}, filesystem = {} }
love.audio.newSource = function(a) return { from = a } end
love.filesystem.newFileData = function(bytes, name) return { name = name } end
love.graphics.newImage = function(a)
  local size = type(a) == "string" and game_sizes[a] or mod_sizes[a.name]
  local img = newproxy(proto)
  dims[img] = { size[1], size[2] }
  return img
end
drawn = nil
love.graphics.draw = function(...) drawn = { n = select('#', ...), ... } end
local printed = {}
print = function(s) printed[#printed + 1] = tostring(s) end
"#;
        let game = format!(
            r#"
top_m1 = love.graphics.newImage("sprites/top_m1.png")
side_m1 = love.graphics.newImage("sprites/side_m1.png")
top_big = love.graphics.newImage("sprites/top_big.png")
top_half = love.graphics.newImage("sprites/top_half.png")
top_copy = love.graphics.newImage("sprites/top_copy.png")
local plain = top_big
local row = function(rel, cat, stem) return '"' .. rel .. '"\t"' .. cat .. '"\t"' .. stem .. '"\t"png"\t""\n' end

-- An image at its pixel size is drawn with the arguments it was given.
love.graphics.draw(plain, 1, 2, 0.5, 3)
assert(drawn.n == 5 and drawn[1] == plain and drawn[5] == 3, "untouched draw")
local kx, ky = QM.Sprites.scaleOf(plain)
assert(kx == 1 and ky == 1)

-- A 400x160 file replaces the 100x40 image: it reports and draws at 100x40.
local old = top_m1
__qm_auto_asset_manifest("{root}/moda",
  row("assets/sprites/top_m1.png", "sprites", "top_m1") .. row("assets/sprites/side_m1.png", "sprites", "side_m1"), "moda")
assert(top_m1 ~= old, "the image was replaced")
assert(top_m1:getWidth() == 100 and top_m1:getHeight() == 40, "it reports the original's size")
local w, h = top_m1:getDimensions()
assert(w == 100 and h == 40)
local rw, rh = QM.Sprites.rawSize(top_m1)
assert(rw == 400 and rh == 160, "rawSize is the file's own size")
kx, ky = QM.Sprites.scaleOf(top_m1)
assert(kx == 0.25 and ky == 0.25)
love.graphics.draw(top_m1, 10, 20, 0, 2, 2, 50, 20)
assert(drawn[1] == top_m1 and drawn[2] == 10 and drawn[3] == 20, "position is unchanged")
assert(drawn[5] == 0.5 and drawn[6] == 0.5, "the scale is multiplied by the image's")
assert(drawn[7] == 200 and drawn[8] == 80, "the origin is given in the file's own pixels")
love.graphics.draw(top_m1, 10, 20)
assert(drawn[5] == 0.25 and drawn[6] == 0.25 and drawn[7] == 0, "defaults are filled in")
love.graphics.draw(top_m1, 5, 5, 0, 2)
assert(drawn[5] == 0.5 and drawn[6] == 0.5, "sy follows sx when left out")
local quad = newproxy()
love.graphics.draw(top_m1, quad, 10, 20, 0, 2)
assert(drawn[2] == quad and drawn[6] == 2, "a draw with a Quad is left alone")

-- Another shape: stretched to the original's box, with a warning.
assert(side_m1:getWidth() == 60 and side_m1:getHeight() == 30)
local warned = false
for _, line in ipairs(printed) do if line:find("side_m1", 1, true) and line:find("another shape", 1, true) then warned = true end end
assert(warned, "a replacement of another shape is reported")

-- The game loads the file again: the replacement comes back, at the same shown size.
local again = love.graphics.newImage("sprites/top_m1.png")
assert(QM.Sprites.rawSize(again) == 400 and again:getWidth() == 100, "a reload keeps the size")

-- [sprites] rules, as the loader writes them.
__qm_sprite_rules("moda", {table})
__qm_apply_sprite_rules()
assert(top_big:getWidth() == 150 and top_big:getHeight() == 100, "size")
assert(top_half:getWidth() == 200 and top_half:getHeight() == 100, "scale")
assert(top_copy:getWidth() == 150 and top_copy:getHeight() == 100, "like takes the other's final size")
local missing = false
for _, line in ipairs(printed) do if line:find("no_such_image", 1, true) then missing = true end end
assert(missing, "a rule for an image the game does not have is reported")

-- From code.
assert(QM.Sprites.setScale(top_big, 1) and top_big:getWidth() == 300, "back at its pixels")
kx = QM.Sprites.scaleOf(top_big)
assert(kx == 1)
assert(QM.Sprites.setSize(top_big, 30, 60) and top_big:getHeight() == 60)
assert(QM.Sprites.matchSize(top_big, top_m1) and top_big:getWidth() == 100)
QM.Sprites.reset(top_big)
assert(top_big:getWidth() == 300)
assert(QM.Sprites.setSize(top_big, 0, 5) == false and QM.Sprites.setSize({{}}, 5, 5) == false, "bad input is refused")
return "ok"
"#
        );
        let chunk = format!("{stubs}\n{}\n{game}", crate::patch::loader::STEAM_SAFETY_PAYLOAD);
        assert_eq!(c.run_chunk("prelude_sprites", &chunk), Ok(Some("ok".into())));
    }

    #[test]
    fn run_chunk_returns_results_and_errors() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        assert_eq!(c.run_chunk("t", "return 'a' .. string.rep('b', 2)"), Ok(Some("abb".into())));
        assert_eq!(c.run_chunk("t", "local x = 1"), Ok(None));
        let err = c.run_chunk("t", "error('no luck')").unwrap_err();
        assert!(err.contains("no luck"), "{err}");
        assert!(c.run_chunk("t", "if then").is_err());
    }

    #[test]
    fn the_footer_reinstalls_the_crash_handler_after_love_load() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let mod_handler = r#"love.errorhandler = function(msg)
    handled[#handled + 1] = "mod: " .. msg
    return "mod handler"
  end"#;
        let checks = r#"
local wrapper = love.errorhandler
assert(wrapper ~= game_handler and love.errhand == wrapper, "the wrapper is installed")
assert(love.load ~= game_load, "love.load is wrapped")
local sum, second = love.load(2, 3)
assert(sum == 5 and second == "second", "love.load passes its results through")
assert(love.errorhandler == wrapper and love.errhand == wrapper, "the wrapper is back on top")
assert(love.errorhandler("boom") == "mod handler", "it chains to the mod's handler")
assert(#handled == 1, "one handler ran")
assert(handled[1]:find("mod: boom", 1, true) == 1, handled[1])
assert(handled[1]:find("[QuarterMaster Crash Attribution]\npatch X", 1, true), handled[1])
assert(calls.crash == 1, "the crash was recorded")
"#;
        assert_eq!(run_footer(&c, "", mod_handler, checks), Ok(Some("ok".into())));

        // A love.load that leaves the handler alone keeps the game's handler in the chain.
        let checks = r#"
love.load(1, 1)
assert(love.errorhandler("boom") == "game handler")
assert(handled[1]:find("game: boom", 1, true) == 1, handled[1])
"#;
        assert_eq!(run_footer(&c, "", "", checks), Ok(Some("ok".into())));

        // So does a mod that sets only the old name, love.errhand.
        let errhand = r#"love.errhand = function(msg)
    return "errhand"
  end"#;
        let checks = r#"
local wrapper = love.errorhandler
love.load(1, 1)
assert(love.errorhandler == wrapper and love.errhand == wrapper)
assert(love.errorhandler("boom") == "errhand")
"#;
        assert_eq!(run_footer(&c, "", errhand, checks), Ok(Some("ok".into())));
    }

    #[test]
    fn the_footer_marks_only_a_quit_that_goes_ahead_as_clean() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let checks = r#"
assert(love.quit ~= game_quit, "love.quit is wrapped")
love.load(1, 1)
assert(love.quit() == nil and calls.clean == 1, "nil: the quit goes ahead")
quit_result = false
assert(love.quit() == false and calls.clean == 2, "false: the quit goes ahead")
quit_result = true
assert(love.quit() == true and calls.clean == 2, "true cancels the quit: not clean")
assert(select('#', love.quit()) == 1, "one result, as the game's love.quit")
quit_fails = true
local ok, res = xpcall(love.quit, function(m) return love.errorhandler(m) end)
assert(not ok and res == "game handler", "the error reached the handler")
assert(handled[#handled]:find("quit failed", 1, true), handled[#handled])
assert(calls.clean == 2, "an error in love.quit is not a clean exit")
assert(calls.crash == 1, "the crash was recorded")
"#;
        assert_eq!(run_footer(&c, "", "", checks), Ok(Some("ok".into())));
    }

    #[test]
    fn the_footer_rewraps_a_love_quit_set_in_love_load() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let mod_quit = r#"love.quit = function()
    calls.mod_quit = calls.mod_quit + 1
    return false
  end"#;
        let checks = r#"
local wrapper = love.quit
love.load(1, 1)
assert(love.quit == wrapper, "the wrapper is back")
assert(love.quit() == false, "the mod's result")
assert(calls.mod_quit == 1 and calls.clean == 1, "the mod's love.quit ran, then the clean mark")
"#;
        assert_eq!(run_footer(&c, "", mod_quit, checks), Ok(Some("ok".into())));
    }

    #[test]
    fn the_footer_runs_a_wrapping_mod_handler_and_the_games_once() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        // The usual idiom: inside love.load, `prev` is the wrapper.
        let wrap = r#"local prev = love.errorhandler
  love.errorhandler = function(msg)
    calls.mod_err = calls.mod_err + 1
    if calls.mod_err > 10 then
      error("LOOP")
    end
    return prev(msg)
  end"#;
        let checks = r#"
local wrapper = love.errorhandler
love.load(1, 1)
assert(love.errorhandler == wrapper, "the wrapper is back on top")
assert(love.errorhandler("boom") == "game handler", "the game's handler ends the chain")
assert(calls.mod_err == 1, "the mod's handler ran once")
assert(#handled == 1 and handled[1]:find("game: boom", 1, true) == 1, handled[1])
assert(handled[1]:find("[QuarterMaster Crash Attribution]", 1, true), handled[1])
assert(calls.crash == 1, "the crash was recorded once")
assert(love.errorhandler("again") == "game handler")
assert(calls.mod_err == 2 and #handled == 2 and calls.crash == 2, "a second error runs the same chain")
"#;
        assert_eq!(run_footer(&c, "", wrap, checks), Ok(Some("ok".into())));
    }

    #[test]
    fn the_footer_runs_a_wrapping_mod_quit_and_the_games_once() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let wrap = r#"local prev = love.quit
  love.quit = function(...)
    calls.mod_quit = calls.mod_quit + 1
    if calls.mod_quit > 10 then
      error("LOOP")
    end
    return prev(...)
  end"#;
        let checks = r#"
local wrapper = love.quit
love.load(1, 1)
assert(love.quit == wrapper, "the wrapper is back")
assert(love.quit() == nil, "the game's result")
assert(calls.mod_quit == 1 and calls.game_quit == 1, "the mod's and the game's love.quit ran once")
assert(calls.clean == 1, "the quit was marked clean once")
quit_result = true
assert(love.quit() == true and calls.game_quit == 2 and calls.clean == 1, "a cancelled quit is not clean")
"#;
        assert_eq!(run_footer(&c, "", wrap, checks), Ok(Some("ok".into())));
    }

    #[test]
    fn the_footer_without_the_qm_module_changes_nothing() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let checks = r#"
assert(love.errorhandler == game_handler and love.quit == game_quit and love.load == game_load)
"#;
        let setup = "package.preload.qm = function() error('no qm') end";
        assert_eq!(run_footer(&c, setup, "", checks), Ok(Some("ok".into())));

        // A qm module without record_clean_exit (an older DLL): the quit still goes ahead.
        let setup = "qm_stub.record_clean_exit = nil";
        let checks = "assert(love.quit() == nil)";
        assert_eq!(run_footer(&c, setup, "", checks), Ok(Some("ok".into())));
    }

    /// The save guard against a stub `love.filesystem` over a real folder. As in LÖVE,
    /// loading a file that does not compile raises an error instead of returning one.
    #[test]
    fn the_save_guard_survives_damaged_saves_and_keeps_the_previous_copy() {
        let Some(c) = compiler() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().replace('\\', "/");
        let stubs = format!(
            r#"
local DIR = "{root}"
local function path(name) return DIR .. "/" .. name end
local function slurp(name)
  local f = io.open(path(name), "rb")
  if not f then return nil end
  local s = f:read("*a")
  f:close()
  return s
end
local function put(name, text)
  local f = assert(io.open(path(name), "wb"))
  f:write(text)
  f:close()
  return true
end
love = {{ filesystem = {{
  getSaveDirectory = function() return DIR end,
  getInfo = function(name) return slurp(name) and {{ type = "file" }} or nil end,
  read = function(name, size)
    local s = slurp(name)
    if s and size then s = s:sub(1, size) end
    return s
  end,
  write = put,
  load = function(name)
    local s = slurp(name)
    if not s then return nil, "Could not open file " .. name end
    local chunk, err = loadstring(s, name)
    if not chunk then error("Syntax error: " .. err .. "\n", 0) end
    return chunk
  end,
}} }}
function newProfile(t) t.fresh = true end
local printed = {{}}
print = function(s) printed[#printed + 1] = tostring(s) end
"#
        );
        let checks = r#"
local fs = love.filesystem
local function save(n) return "local _={}\nreturn {wave=" .. n .. "}" end
local zeros = string.rep("\0", 4096)
local function has(text)
  for _, line in ipairs(printed) do if line:find(text, 1, true) then return true end end
  return false
end

-- A good save loads as before.
put("saveMNB3.lua", save(1))
assert(fs.load("saveMNB3.lua")().wave == 1)
assert(#printed == 0, "nothing is said about a good save")

-- A missing file and a file that is not a save go to LÖVE untouched.
local chunk, err = fs.load("saveMNB2.lua")
assert(chunk == nil and err:find("Could not open"), tostring(err))
put("other.lua", "this is not lua")
assert(not pcall(fs.load, "other.lua"), "another file's error is the game's to see")

-- Zero bytes where the save should be, and no other copy: a fresh profile, no error.
put("saveMNB2.lua", zeros)
local t = fs.load("saveMNB2.lua")()
assert(t.fresh == true and t.wave == nil)
assert(slurp("saveMNB2.lua") == nil and slurp("saveMNB2.lua.damaged") == zeros, "the damaged file is set aside")
assert(has("saveMNB2.lua could not be loaded") and has("fresh profile"))
assert(QM.saveGuard.used[1].file == "saveMNB2.lua" and QM.saveGuard.used[1].from == nil)

-- Each write keeps the copy it replaces.
fs.write("saveMNB2.lua", save(2))
assert(slurp("saveMNB2.lua.prev") == nil, "there was nothing to keep the first time")
fs.write("saveMNB2.lua", save(3))
assert(slurp("saveMNB2.lua") == save(3) and slurp("saveMNB2.lua.prev") == save(2))
fs.write("saveMNB2.lua", save(4))
assert(slurp("saveMNB2.lua.prev") == save(3))

-- The write is lost (zero bytes on disk): the previous copy is loaded.
put("saveMNB2.lua", zeros)
assert(fs.load("saveMNB2.lua")().wave == 3)
assert(has("using saveMNB2.lua.prev in its place"))
-- And a damaged file never replaces a good previous copy.
put("saveMNB2.lua", zeros)
fs.write("saveMNB2.lua", save(5))
assert(slurp("saveMNB2.lua.prev") == save(3) and slurp("saveMNB2.lua") == save(5))

-- saveMNB3.lua: the game's own backup first, then the previous copy.
fs.write("saveMNB3.lua", save(10))
assert(slurp("saveMNB3.lua.prev") == save(1))
put("backUp.lua", save(9))
put("saveMNB3.lua", "return {")
assert(fs.load("saveMNB3.lua")().wave == 9)
put("saveMNB3.lua", zeros)
put("backUp.lua", zeros)
assert(fs.load("saveMNB3.lua")().wave == 1, "backUp.lua is damaged too: the previous copy")
assert(slurp("backUp.lua") == zeros, "only the file that was asked for is set aside")

-- A save that compiles but fails when run, or gives no table.
put("saveMNBclassic.lua", "error('boom')")
assert(fs.load("saveMNBclassic.lua")().fresh == true)
put("saveMNBclassic.lua", "return 5")
assert(fs.load("saveMNBclassic.lua")().fresh == true)

-- Other files are written as before, and get no copy.
fs.write("saveModSettings.lua", "return {}")
fs.write("saveModSettings.lua", "return {a=1}")
assert(slurp("saveModSettings.lua.prev") == nil)
return "ok"
"#;
        let chunk = format!("{stubs}\n{}\n{checks}", include_str!("patch/save_guard.lua"));
        assert_eq!(c.run_chunk("save_guard", &chunk), Ok(Some("ok".into())));

        // The guard is part of the code every modded game starts with, and the menu code
        // no longer carries its own.
        assert!(crate::patch::loader::STEAM_SAFETY_PAYLOAD.contains("QM.saveGuard"));
        assert!(!crate::patch::menu::MENU_RUNTIME.contains("origFsLoad"));
    }
}
