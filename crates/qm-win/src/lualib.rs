use libloading::Library;
use qm_core::sys::LuaLib;

/// Bind lua51.dll. A missing library or symbol is an error naming it.
pub unsafe fn get_lualib() -> anyhow::Result<LuaLib> {
    let library = Library::new("lua51.dll")?;
    LuaLib::from_library(&library).map_err(anyhow::Error::msg)
}
