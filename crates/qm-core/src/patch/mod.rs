use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub use copy::CopyPatch;
pub use module::ModulePatch;
pub use pattern::PatternPatch;
pub use regex::RegexPatch;

pub mod copy;
pub mod loader;
pub mod module;
pub mod pattern;
pub mod regex;
pub mod table;
pub mod vars;

pub type Priority = i32;

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnFailurePolicy {
    Warn,
    SkipPatch,
    SkipMod,
    Abort,
}

impl Default for OnFailurePolicy {
    fn default() -> Self {
        Self::Warn
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Manifest {
    pub version: String,
    // Does nothing, kept for legacy compat
    #[serde(default)]
    pub dump_lua: bool,
    #[serde(default)]
    pub priority: Priority,
}

// Represents a single .toml file after deserialization.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PatchFile {
    pub manifest: Manifest,
    #[serde(default)]
    pub patches: Vec<Patch>,

    // A table of variable name = value bindings. These are interpolated
    // into injected source code as the *last* step in the patching process.
    #[serde(default)]
    pub vars: HashMap<String, String>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum Patch {
    // A patch which applies some change to a series of line(s) after a line with a match
    // to the provided pattern has been found.
    Pattern(PatternPatch),
    Regex(RegexPatch),
    Copy(CopyPatch),
    Module(ModulePatch),
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum Target {
    Single(String),
    Multi(Vec<String>),
}

impl Target {
    pub fn primary(&self) -> &str {
        match self {
            Target::Single(s) => s.as_str(),
            Target::Multi(v) => v.first().map(|s| s.as_str()).unwrap_or(""),
        }
    }
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InsertPosition {
    At,
    Before,
    After,
}

impl Patch {
    pub fn type_name(&self) -> &'static str {
        match self {
            Patch::Module(_) => "Module",
            Patch::Copy(_) => "Copy",
            Patch::Pattern(_) => "Pattern",
            Patch::Regex(_) => "Regex",
        }
    }

    pub fn id(&self) -> Option<&str> {
        match self {
            Patch::Module(m) => m.id.as_deref(),
            Patch::Copy(c) => c.id.as_deref(),
            Patch::Pattern(p) => p.id.as_deref(),
            Patch::Regex(r) => r.id.as_deref(),
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Patch::Module(m) => Some(m.name.as_str()),
            Patch::Copy(c) => c.name.as_deref(),
            Patch::Pattern(p) => p.name.as_deref(),
            Patch::Regex(r) => r.name.as_deref(),
        }
    }

    pub fn description(&self) -> Option<&str> {
        match self {
            Patch::Module(_) => None,
            Patch::Copy(_) => None,
            Patch::Pattern(p) => p.description.as_deref(),
            Patch::Regex(r) => r.description.as_deref(),
        }
    }

    pub fn requires(&self) -> &[String] {
        match self {
            Patch::Module(_) => &[],
            Patch::Copy(_) => &[],
            Patch::Pattern(p) => &p.requires,
            Patch::Regex(r) => &r.requires,
        }
    }

    pub fn mod_id(&self) -> Option<&str> {
        match self {
            Patch::Module(m) => m.mod_id.as_deref(),
            Patch::Copy(c) => c.mod_id.as_deref(),
            Patch::Pattern(p) => p.mod_id.as_deref(),
            Patch::Regex(r) => r.mod_id.as_deref(),
        }
    }

    pub fn priority_override(&self) -> Option<Priority> {
        match self {
            Patch::Module(m) => m.priority,
            Patch::Copy(c) => c.priority,
            Patch::Pattern(p) => p.priority,
            Patch::Regex(r) => r.priority,
        }
    }

    pub fn on_failure(&self) -> OnFailurePolicy {
        match self {
            Patch::Module(m) => m.on_failure,
            Patch::Copy(c) => c.on_failure,
            Patch::Pattern(p) => p.on_failure,
            Patch::Regex(r) => r.on_failure,
        }
    }

    pub fn is_disabled(&self) -> bool {
        match self {
            Patch::Module(m) => m.disabled,
            Patch::Copy(c) => c.disabled,
            Patch::Pattern(p) => p.disabled,
            Patch::Regex(r) => r.disabled,
        }
    }

    pub fn set_disabled(&mut self, disabled: bool) {
        match self {
            Patch::Module(m) => m.disabled = disabled,
            Patch::Copy(c) => c.disabled = disabled,
            Patch::Pattern(p) => p.disabled = disabled,
            Patch::Regex(r) => r.disabled = disabled,
        }
    }

    pub fn can_apply_to_target(&self, target: &str) -> bool {
        match self {
            Patch::Module(m) => m.before.as_deref() == Some(target),
            Patch::Copy(c) => c.target.can_apply(target),
            Patch::Pattern(p) => p.target.can_apply(target),
            Patch::Regex(r) => r.target.can_apply(target),
        }
    }

    pub fn expected_count(&self) -> Option<usize> {
        match self {
            Patch::Module(_) => Some(1),
            Patch::Copy(_) => Some(1),
            Patch::Pattern(p) => p.times,
            Patch::Regex(r) => r.times,
        }
    }

    pub fn primary_target(&self) -> &str {
        match self {
            Patch::Module(m) => m.before.as_deref().unwrap_or("main.lua"),
            Patch::Copy(c) => c.target.primary(),
            Patch::Pattern(p) => p.target.primary(),
            Patch::Regex(r) => r.target.primary(),
        }
    }

    pub fn set_metadata(
        &mut self,
        mod_id: String,
        fallback_id: String,
        comment_name: Option<String>,
    ) {
        match self {
            Patch::Module(m) => {
                m.mod_id = Some(mod_id);
                if m.id.is_none() {
                    m.id = Some(fallback_id);
                }
            }
            Patch::Copy(c) => {
                c.mod_id = Some(mod_id);
                if c.id.is_none() {
                    c.id = Some(fallback_id.clone());
                }
                if c.name.is_none() {
                    c.name = comment_name.or(Some(fallback_id));
                }
            }
            Patch::Pattern(p) => {
                p.mod_id = Some(mod_id);
                if p.id.is_none() {
                    p.id = Some(fallback_id.clone());
                }
                if p.name.is_none() {
                    p.name = comment_name.or(Some(fallback_id));
                }
            }
            Patch::Regex(r) => {
                r.mod_id = Some(mod_id);
                if r.id.is_none() {
                    r.id = Some(fallback_id.clone());
                }
                if r.name.is_none() {
                    r.name = comment_name.or(Some(fallback_id));
                }
            }
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Patch::Module(m) => format!(
                "Module: '{}' (before: {:?}, load_now: {})",
                m.name, m.before, m.load_now
            ),
            Patch::Copy(c) => format!("Copy ({:?}) -> target: {:?}", c.position, c.target),
            Patch::Pattern(p) => {
                let snippet = p.pattern.lines().next().unwrap_or("").trim();
                format!(
                    "Pattern ({:?}) -> target: {:?} ['{}']",
                    p.position, p.target, snippet
                )
            }
            Patch::Regex(r) => {
                let snippet = r.pattern.lines().next().unwrap_or("").trim();
                format!(
                    "Regex ({:?}) -> target: {:?} ['{}']",
                    r.position, r.target, snippet
                )
            }
        }
    }
}
