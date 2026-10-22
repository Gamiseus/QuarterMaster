use libloading::{Error, Library};
use qm_core::sys::LuaLib;

pub unsafe fn get_lualib() -> Result<LuaLib, Error> {
    let library = Library::new("lua51.dll")?;
    Ok(LuaLib::from_library(&library))
}
