#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
use log::error;
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_void, CString};
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use itertools::Itertools;
use libloading::Library;

use log::info;

pub static LUA: OnceLock<LuaLib> = OnceLock::new();

pub type LuaState = c_void;
pub type LuaFunc = unsafe extern "C" fn(*mut LuaState) -> c_int;

pub const LUA_GLOBALSINDEX: c_int = -10002;
pub const LUA_TNIL: c_int = 0;
pub const LUA_TBOOLEAN: c_int = 1;
pub const fn lua_upvalueindex(i: c_int) -> c_int {
    // This is a macro in lua
    LUA_GLOBALSINDEX - i
}

/// What a Lua wrapper returns when `LUA` is unset: 0, null or nothing.
pub trait LuaZero {
    fn zero() -> Self;
}

impl LuaZero for () {
    fn zero() -> Self {}
}

impl LuaZero for c_int {
    fn zero() -> Self {
        0
    }
}

impl<T> LuaZero for *const T {
    fn zero() -> Self {
        ptr::null()
    }
}

static LUA_MISSING_LOGGED: AtomicBool = AtomicBool::new(false);

/// `LUA` is set before any wrapper runs (`QuarterMaster::init`); this is a safety net.
fn lua_missing(method: &str) {
    if !LUA_MISSING_LOGGED.swap(true, Ordering::Relaxed) {
        error!("QuarterMaster: Lua call {method} before the Lua library was bound; it did nothing.");
    }
}

/// The DLL's export name of a wrapper: `lual_x` is `luaL_x`.
fn symbol_name(method: &str) -> String {
    match method.strip_prefix("lual_") {
        Some(rest) => format!("luaL_{rest}"),
        None => method.to_string(),
    }
}

/// The bytes Lua returned, or none for a null pointer.
unsafe fn lua_bytes<'a, T>(ptr: *const T, len: usize) -> &'a [u8] {
    if ptr.is_null() {
        return &[];
    }
    slice::from_raw_parts(ptr as *const u8, len)
}

/// `s` as a C string; interior NUL bytes are dropped.
fn to_cstring(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|e| {
        let mut bytes = e.into_vec();
        bytes.retain(|&b| b != 0);
        CString::new(bytes).unwrap_or_default()
    })
}

macro_rules! generate {
    ($libname:ident {
        $(
            $vis:vis unsafe extern "C" fn $method:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)?;
        )*
    }) => {
        #[repr(C)]
        pub struct $libname {
            $(
                $vis $method: unsafe extern "C" fn($($arg: $ty),*) $(-> $ret)?,
            )*
        }

        impl $libname {
            /// Bind every function through `find` (export name to address). A missing or null
            /// symbol is an error naming it.
            /// # Safety
            /// Each address `find` returns must be the named Lua C function.
            pub unsafe fn from_lookup(
                mut find: impl FnMut(&str) -> Option<*const c_void>,
            ) -> Result<Self, String> {
                Ok($libname {
                    $(
                        $method: {
                            let name = symbol_name(stringify!($method));
                            let addr = find(&name)
                                .filter(|a| !a.is_null())
                                .ok_or_else(|| format!("lua51.dll has no {name}"))?;
                            std::mem::transmute::<
                                *const c_void,
                                unsafe extern "C" fn($($ty),*) $(-> $ret)?,
                            >(addr)
                        },
                    )*
                })
            }
        }

        $(
            /// # Safety
            $vis unsafe extern "C" fn $method($($arg: $ty),*) $(-> $ret)? {
                match LUA.get() {
                    Some(lua) => (lua.$method)($($arg),*),
                    None => {
                        lua_missing(stringify!($method));
                        LuaZero::zero()
                    }
                }
            }
        )*
    };
}

// TODO: Can we make this work with variable number of upvalues?
unsafe extern "C" fn lua_return_values(state: *mut LuaState) -> c_int {
    let index = lua_upvalueindex(1);
    lua_pushvalue(state, index);
    1
}

// HACK: Panics if not inlined?
#[inline(always)]
pub(crate) unsafe fn check_lua_string(state: *mut LuaState, index: c_int) -> String {
    let mut str_len = 0usize;
    let arg_str = lual_checklstring(state, index, &mut str_len);

    String::from_utf8_lossy(lua_bytes(arg_str, str_len)).to_string()
}

pub(crate) unsafe fn opt_lua_string(state: *mut LuaState, index: c_int) -> Option<String> {
    if lua_gettop(state) < index {
        return None;
    }
    let t = lua_type(state, index);
    if t == LUA_TNIL || t < 0 {
        return None;
    }
    Some(check_lua_string(state, index))
}

generate! (LuaLib {
    pub unsafe extern "C" fn lua_call(state: *mut LuaState, nargs: c_int, nresults: c_int);
    pub unsafe extern "C" fn lua_pcall(state: *mut LuaState, nargs: c_int, nresults: c_int, errfunc: c_int) -> c_int;
    pub unsafe extern "C" fn lua_getfield(state: *mut LuaState, index: c_int, k: *const c_char);
    pub unsafe extern "C" fn lua_setfield(state: *mut LuaState, index: c_int, k: *const c_char);
    pub unsafe extern "C" fn lua_gettop(state: *mut LuaState) -> c_int;
    pub unsafe extern "C" fn lua_settop(state: *mut LuaState, index: c_int);
    pub unsafe extern "C" fn lua_pushvalue(state: *mut LuaState, index: c_int);
    pub unsafe extern "C" fn lua_pushcclosure(state: *mut LuaState, f: LuaFunc, n: c_int);
    pub unsafe extern "C" fn lua_tolstring(state: *mut LuaState, index: c_int, len: *mut usize) -> *const c_char;
    pub unsafe extern "C" fn lua_type(state: *mut LuaState, index: c_int) -> c_int;
    pub unsafe extern "C" fn lua_pushstring(state: *mut LuaState, string: *const char);
    pub unsafe extern "C" fn lua_pushnumber(state: *mut LuaState, number: f64);
    pub unsafe extern "C" fn lua_pushboolean(state: *mut LuaState, b: c_int);
    pub unsafe extern "C" fn lua_toboolean(state: *mut LuaState, index: c_int) -> c_int;
    pub unsafe extern "C" fn lua_settable(state: *mut LuaState, index: c_int);
    pub unsafe extern "C" fn lua_createtable(state: *mut LuaState, narr: c_int, nrec: c_int);
    pub unsafe extern "C" fn lua_error(state: *mut LuaState) -> c_int;
    pub unsafe extern "C" fn lual_register(state: *mut LuaState, libname: *const char, l: *const c_void);
    pub unsafe extern "C" fn lual_checklstring(state: *mut LuaState, index: c_int, len: *mut usize) -> *const char;
});

impl LuaLib {
    /// Construct a LuaLib from a loaded library. A missing symbol is an error naming it.
    /// # Safety
    /// The library must define Lua symbols.
    pub unsafe fn from_library(library: &Library) -> Result<Self, String> {
        Self::from_lookup(|name| {
            library
                .get::<*const c_void>(name.as_bytes())
                .ok()
                .map(|s| *s)
        })
    }
}

// TODO: implement all lua methods on this(?)
pub(crate) trait LuaStateTrait {
    unsafe fn push<P: Pushable>(self, obj: P);
    unsafe fn push_closure(self, func: LuaFunc, vals: c_int);
    unsafe fn to_string(self, index: c_int) -> String;
}

impl LuaStateTrait for *mut LuaState {
    unsafe fn push<P: Pushable>(self, obj: P) {
        obj.push(self)
    }

    unsafe fn push_closure(self, func: LuaFunc, vals: c_int) {
        lua_pushcclosure(self, func, vals);
    }

    unsafe fn to_string(self, index: c_int) -> String {
        let mut str_len = 0usize;
        let arg_str = lua_tolstring(self, index, &mut str_len);

        String::from_utf8_lossy(lua_bytes(arg_str, str_len)).to_string()
    }
}
/// A trait which allows the implementing value to generically push its value onto the Lua stack.
pub trait Pushable {
    /// Push this value onto the Lua stack.
    ///
    /// # Safety
    /// Directly interacts with native Lua state.
    unsafe fn push(&self, state: *mut LuaState);
}

impl Pushable for String {
    unsafe fn push(&self, state: *mut LuaState) {
        let value = format!("{self}\0");
        lua_pushstring(state, value.as_ptr() as _);
    }
}

impl Pushable for &String {
    unsafe fn push(&self, state: *mut LuaState) {
        let value = format!("{self}\0");
        lua_pushstring(state, value.as_ptr() as _);
    }
}

impl Pushable for &str {
    unsafe fn push(&self, state: *mut LuaState) {
        let value = to_cstring(self);
        lua_pushstring(state, value.into_raw() as _);
    }
}

impl Pushable for isize {
    unsafe fn push(&self, state: *mut LuaState) {
        lua_pushnumber(state, *self as _);
    }
}

impl Pushable for bool {
    unsafe fn push(&self, state: *mut LuaState) {
        lua_pushboolean(state, *self as _);
    }
}

impl Pushable for LuaFunc {
    unsafe fn push(&self, state: *mut LuaState) {
        lua_pushcclosure(state, *self as _, 0);
    }
}

pub struct LuaVar<P>
where
    P: std::ops::Deref,
    P::Target: Pushable,
{
    name: String,
    val: P,
}

pub struct LuaTable {
    var: Vec<LuaVar<Box<dyn Pushable>>>,
}

impl LuaTable {
    pub(crate) fn new() -> Self {
        LuaTable { var: vec![] }
    }

    /// Add a variable to this Lua module.
    pub fn add_var<P: Pushable + 'static>(self, name: &'static str, val: P) -> Self {
        let name = format!("{name}\0");
        let mut var = self.var;
        let val = Box::new(val);
        var.push(LuaVar { name, val });

        LuaTable { var }
    }
}

impl Pushable for LuaTable {
    unsafe fn push(&self, state: *mut LuaState) {
        // Create a table at the top of the stack.
        // The size is a hint only.
        lua_createtable(state, 0, self.var.len().try_into().unwrap_or(0));

        for lua_var in self.var.iter() {
            // Push the var name and value onto the stack.
            lua_pushstring(state, lua_var.name.as_ptr() as _);
            lua_var.val.push(state);

            // Set the table key:val from what we previously pushed onto the stack.
            lua_settable(state, -3);
        }
    }
}

/// Commit this Lua module to native Lua state.
///
/// # Safety
/// Directly interacts and mutates native Lua state.
pub unsafe fn preload_module<P: Pushable>(state: *mut LuaState, name: &'static str, value: P) {
    let top = lua_gettop(state);

    // Get the package.preloads
    lua_getfield(state, LUA_GLOBALSINDEX, c"package".as_ptr());
    lua_getfield(state, -1, c"preload".as_ptr());
    let preload_index = lua_gettop(state);

    // Push the value to the stack
    value.push(state);

    // Push our function to the stack
    let func: LuaFunc = lua_return_values;

    state.push_closure(func, 1);

    let name = to_cstring(name);
    lua_setfield(state, preload_index, name.into_raw() as _);

    // Reset the stack.
    lua_settop(state, top);
}

/// Load the provided buffer as a lua module with the specified name.
/// # Safety
/// Makes a lot of FFI calls, mutates internal C lua state.
pub unsafe fn load_module<F: Fn(*mut LuaState, *const u8, usize, *const u8, *const u8) -> u32>(
    state: *mut LuaState,
    name: &str,
    buffer: &str,
    lual_loadbufferx: &F,
) {
    let p_name = format!("@{name}");
    let p_name_cstr = to_cstring(&p_name);

    // Push the global package.preload table onto the top of the stack, saving its index.
    let stack_top = lua_gettop(state);
    lua_getfield(state, LUA_GLOBALSINDEX, c"package".as_ptr());
    lua_getfield(state, -1, c"preload".as_ptr());

    // This is the index of the `package.loaded` table.
    let field_index = lua_gettop(state);

    // Load the buffer and execute it via lua_pcall, pushing the result to the top of the stack.
    lual_loadbufferx(
        state,
        buffer.as_ptr(),
        buffer.len(),
        p_name_cstr.into_raw() as _,
        ptr::null(),
    );

    let lua_pcall_return = lua_pcall(state, 0, -1, 0);
    if lua_pcall_return == 0 {
        lua_pushcclosure(state, lua_identity_closure, 1);
        // Insert wrapped pcall results onto the package.preload global table.
        let module_cstr = to_cstring(name);

        lua_setfield(state, field_index, module_cstr.into_raw());
    }

    lua_settop(state, stack_top);
}

// Checks if a module is in the preload table. Used to check if QuarterMaster was already initialized.
// # Safety
// Uses the native Lua API.
#[allow(dead_code)]
pub(crate) unsafe fn is_module_preloaded(state: *mut LuaState, name: &str) -> bool {
    let name_cstr = to_cstring(name);
    let stack_top = lua_gettop(state);
    lua_getfield(state, LUA_GLOBALSINDEX, c"package".as_ptr());
    lua_getfield(state, -1, c"preload".as_ptr());
    lua_getfield(state, -1, name_cstr.as_ptr());

    let res = lua_type(state, -1) != LUA_TNIL;

    lua_settop(state, stack_top);
    res
}

/// An override print function, copied piecemeal from the Lua 5.1 source, but in Rust.
/// # Safety
/// Native lua API access. It's unsafe, it's unchecked, it will probably eat your firstborn.
pub unsafe extern "C" fn override_print(state: *mut LuaState) -> c_int {
    crate::guard::unguarded(|| {
        let argc = lua_gettop(state);
        let mut out = VecDeque::new();

        for _ in 0..argc {
            // We call Lua's builtin tostring function because we don't have access to the 5.3 luaL_tolstring
            // helper function. It's not pretty, but it works.
            lua_getfield(state, LUA_GLOBALSINDEX, c"tostring".as_ptr());
            lua_pushvalue(state, -2);
            lua_call(state, 1, 1);

            let mut str_len = 0usize;
            let arg_str = lua_tolstring(state, -1, &mut str_len);

            let arg_str = String::from_utf8_lossy(lua_bytes(arg_str, str_len)).to_string();

            out.push_front(arg_str);
            lua_settop(state, -3);
        }

        let msg = out.into_iter().join("\t");

        let lower = msg.to_lowercase();
        if lower.contains("error") || lower.contains("traceback") || lower.contains("attempt to ") {
            error!("[G] {msg}");
        } else {
            info!("[G] {msg}");
        }

        0
    })
}

// The closures below only make Lua calls, so they skip `guard::unguarded`
// (lua_err_identity_closure always longjmps, which would skip the restore).

/// A function, which as a Lua closure, returns the first upvalue. This lets it
/// be used to wrap lua values into a closure which returns that value.
/// # Safety
/// Makes some FFI calls, mutates internal C lua state.
pub unsafe extern "C" fn lua_identity_closure(state: *mut LuaState) -> c_int {
    // LUA_GLOBALSINDEX - 1 is where the first upvalue is located
    lua_pushvalue(state, LUA_GLOBALSINDEX - 1);
    // We just return that value
    1
}

/// A function, which as a Lua closure, throws the first upvalue. This lets it
/// be used to wrap lua values into a closure which throws that value.
/// # Safety
/// Makes some FFI calls, mutates internal C lua state.
pub unsafe extern "C" fn lua_err_identity_closure(state: *mut LuaState) -> c_int {
    // LUA_GLOBALSINDEX - 1 is where the first upvalue is located
    lua_pushvalue(state, LUA_GLOBALSINDEX - 1);
    lua_error(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    unsafe extern "C" fn stub(_: *mut LuaState) -> c_int {
        0
    }

    const NAMES: &[&str] = &[
        "lua_call",
        "lua_pcall",
        "lua_getfield",
        "lua_setfield",
        "lua_gettop",
        "lua_settop",
        "lua_pushvalue",
        "lua_pushcclosure",
        "lua_tolstring",
        "lua_type",
        "lua_pushstring",
        "lua_pushnumber",
        "lua_pushboolean",
        "lua_toboolean",
        "lua_settable",
        "lua_createtable",
        "lua_error",
        "luaL_register",
        "luaL_checklstring",
    ];

    fn table(without: Option<&str>) -> HashMap<String, *const c_void> {
        NAMES
            .iter()
            .filter(|n| Some(**n) != without)
            .map(|n| (n.to_string(), stub as LuaFunc as *const c_void))
            .collect()
    }

    #[test]
    fn from_lookup_binds_every_symbol() {
        let syms = table(None);
        let lib = unsafe { LuaLib::from_lookup(|n| syms.get(n).copied()) };
        assert!(lib.is_ok());
    }

    #[test]
    fn from_lookup_names_a_missing_symbol() {
        for missing in ["lua_settable", "luaL_checklstring"] {
            let syms = table(Some(missing));
            let err = unsafe { LuaLib::from_lookup(|n| syms.get(n).copied()) }
                .err()
                .unwrap();
            assert_eq!(err, format!("lua51.dll has no {missing}"));
        }
    }

    #[test]
    fn from_lookup_rejects_a_null_symbol() {
        let mut syms = table(None);
        syms.insert("lua_type".to_string(), ptr::null());
        let err = unsafe { LuaLib::from_lookup(|n| syms.get(n).copied()) }
            .err()
            .unwrap();
        assert_eq!(err, "lua51.dll has no lua_type");
    }

    #[test]
    fn to_cstring_drops_interior_nul() {
        assert_eq!(to_cstring("a\0b").as_bytes(), b"ab");
        assert_eq!(to_cstring("qm").as_bytes(), b"qm");
    }

    #[test]
    fn lua_bytes_of_null_is_empty() {
        assert!(unsafe { lua_bytes(ptr::null::<c_char>(), 5) }.is_empty());
    }
}
