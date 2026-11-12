#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::{
    ffi::CString,
    path::{Path, PathBuf},
    ptr,
};

use super::{OnFailurePolicy, Priority};
use crate::prepare::canonical_path;
use crate::sys::{self, lua_err_identity_closure, lua_identity_closure, LuaState, LuaStateTrait};
use crate::RUNTIME;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ModulePatch {
    pub source: PathBuf,
    // Only has meaning if `load_now` is true. Evaluate the module immediately before
    // this file.
    #[serde(default)]
    pub before: Option<String>,
    pub name: String,

    // If enabled, evaluate the module immediately upon loading it
    #[serde(default)]
    pub load_now: bool,

    pub id: Option<String>,
    pub priority: Option<Priority>,
    #[serde(default)]
    pub on_failure: OnFailurePolicy,

    #[serde(skip)]
    pub mod_id: Option<String>,

    #[serde(skip)]
    pub disabled: bool,

    /// Profile and compat data (Phase C §3.3); never read from a patch file.
    #[serde(skip)]
    pub qm: crate::patch::plan::PatchExtra,

    // Used for display name of the source. Is the relative path.
    #[serde(skip)]
    pub display_source: String,

    // Preloaded at load time
    #[serde(skip)]
    pub content: String,
}

impl ModulePatch {
    /// Whether a `load_now` module runs before chunk `file_name`: its `before` names the same
    /// file in canonical form (D44).
    pub fn runs_before(&self, file_name: &str) -> bool {
        self.before
            .as_deref()
            .is_some_and(|b| canonical_path(b) == canonical_path(file_name))
    }

    /// The chunk name the game loads this module under: `=[qm <name> "<source>"]` (D44).
    pub fn chunk_name(&self) -> String {
        format!("=[qm {} \"{}\"]", self.name, self.display_source)
    }

    /// Apply a module patch by loading the input file(s) into memory and injecting them into
    /// the global `package.preload` table.
    ///
    /// # Safety
    /// This function is unsafe as it interfaces directly with a series of dynamically loaded
    /// native lua functions.
    pub unsafe fn apply(
        &self,
        file_name: &str,
        state: *mut LuaState,
        path: &Path,
    ) -> Result<bool, String> {
        // Stop if we're not at the correct insertion point (canonical paths, D44).
        if self.load_now && !self.runs_before(file_name) {
            return Ok(false);
        }

        let source = self.content.as_bytes();
        let source_len = source.len();

        // The loader rejects a NUL in the name or source; these errors are a backstop.
        let name_cstr = CString::new(self.chunk_name())
            .map_err(|_| format!("Module {:?} has a NUL in its name or source", self.name))?;
        let module_cstr = CString::new(self.name.clone())
            .map_err(|_| format!("Module {:?} has a NUL in its name", self.name))?;
        let Some(qm) = RUNTIME.get() else {
            return Err("Internal QuarterMaster error: Failed to acquire the QM runtime".to_owned());
        };

        // Push the global package.preload table onto the top of the stack, saving its index.
        let stack_top = sys::lua_gettop(state);
        sys::lua_getfield(state, sys::LUA_GLOBALSINDEX, c"package".as_ptr() as _);
        sys::lua_getfield(state, -1, c"preload".as_ptr() as _);

        // This is the index of the `package.preload` table.
        let field_index = sys::lua_gettop(state);

        // Load the buffer and execute it via lua_pcall, pushing the result to the top of the stack.
        let return_code = qm.apply_buffer_patches(
            state,
            source.as_ptr(),
            source_len,
            name_cstr.into_raw() as _,
            ptr::null(),
        );

        if return_code != 0 {
            log::error!(
                "Failed to load module {} for module patch from {}:",
                self.name,
                path.display()
            );
            let err = state.to_string(-1);
            log::error!("Error: {err}");
            state.push_closure(lua_err_identity_closure, 1);
            sys::lua_setfield(state, field_index, module_cstr.into_raw() as _);
            sys::lua_settop(state, stack_top);
            if self.load_now {
                return Err(
                    "An error occured loading a load_now module:\n\nError: ".to_owned() + &err
                );
            } else {
                return Ok(false);
            }
        }

        if self.load_now {
            // Evaluate the results of the buffer now
            let return_code = sys::lua_pcall(state, 0, 1, 0);
            if return_code != 0 {
                log::error!(
                    "Evaluation of module {} failed for module patch from {}:",
                    self.name,
                    path.display()
                );
                let err = state.to_string(-1);
                log::error!("Error: {err}");
                state.push_closure(lua_err_identity_closure, 1);
                sys::lua_setfield(state, field_index, module_cstr.into_raw() as _);
                sys::lua_settop(state, stack_top);
                return Err(
                    "An error occured evaluating a load_now module:\n\nError: ".to_owned() + &err
                );
            }
            // Wrap this in the identity closure function
            state.push_closure(lua_identity_closure, 1);
        }

        // Insert results onto the package.preload global table.
        sys::lua_setfield(state, field_index, module_cstr.into_raw() as _);
        // Always ensure that the lua stack is in good order
        sys::lua_settop(state, stack_top);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(before: &str) -> ModulePatch {
        serde_json::from_value(serde_json::json!({
            "source": "x.lua", "name": "m", "load_now": true, "before": before
        }))
        .unwrap()
    }

    #[test]
    fn load_now_matches_canonical_paths() {
        // Output change (D44, Task 12): before Phase D a `before` key matched only the chunk
        // name with its `@` stripped, exactly, so only "main.lua" ran the module. Every other
        // form here, "@main.lua" and "=main.lua" among them, is a new match.
        for before in ["main.lua", "@main.lua", "=main.lua", "./main.lua", ".\\main.lua"] {
            for chunk in ["main.lua", "@main.lua", "@./main.lua"] {
                assert!(module(before).runs_before(chunk), "{before} {chunk}");
            }
        }
        assert!(!module("main.lua").runs_before("@x/main.lua"));
        assert!(!module("main.lua").runs_before("@conf.lua"));
    }
}
