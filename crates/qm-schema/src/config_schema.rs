use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::save_parser::{SerDocument, SerValue};

#[cfg_attr(feature = "specta", derive(specta::Type))]
/// Scalar value for a mod configuration option.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigValue {
    Bool(bool),
    Number(f64),
    String(String),
}

impl ConfigValue {
    pub fn as_display_string(&self) -> String {
        match self {
            ConfigValue::Bool(b) => b.to_string(),
            ConfigValue::Number(n) => {
                if n.fract() == 0.0 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            ConfigValue::String(s) => s.clone(),
        }
    }
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
/// Declarative visibility rule (`showIf = { id = "event_select", value = "Halloween" }`)
/// or inferred from a local helper function in `config.lua`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ShowIfRule {
    pub option_id: String,
    pub equals: Vec<ConfigValue>,
}

#[derive(Deserialize)]
struct ShowIfHelper {
    #[serde(alias = "id")]
    option_id: String,
    #[serde(default)]
    value: Option<ConfigValue>,
    #[serde(default)]
    values: Option<Vec<ConfigValue>>,
    #[serde(default)]
    equals: Option<Vec<ConfigValue>>,
}

impl<'de> Deserialize<'de> for ShowIfRule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let helper = ShowIfHelper::deserialize(deserializer)?;
        let mut equals = helper.equals.or(helper.values).unwrap_or_default();
        if let Some(val) = helper.value {
            if !equals.contains(&val) {
                equals.push(val);
            }
        }
        Ok(ShowIfRule {
            option_id: helper.option_id,
            equals,
        })
    }
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
/// Normalized option control type for the Launcher UI and `GMenu`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConfigOptionType {
    #[default]
    Checkbox,
    Cycle,
    Slider,
    Separator,
    Button,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigOptionDef {
    pub id: String,
    pub label: String,
    #[serde(alias = "type")]
    pub option_type: ConfigOptionType,
    pub default: Option<ConfigValue>,
    pub description: Option<String>,
    pub options: Vec<ConfigValue>,
    pub labels: Vec<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub show_if: Option<ShowIfRule>,
    pub button_text: Option<String>,
    pub aliases: Vec<String>,
    /// For a `button`: the global function the game calls on a click, by name. A dotted name
    /// (`ExampleMod.run`) is looked up one level into that global table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_click: Option<String>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigModeDef {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub priority: Option<i32>,
    pub on_click: Option<String>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigPageDef {
    pub id: String,
    pub title: String,
    pub page_type: Option<String>,
    pub content: Option<String>,
    pub action_label: Option<String>,
    pub action_on_click: Option<String>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeybindDef {
    pub id: String,
    pub name: String,
    #[serde(default = "default_keybind_category")]
    pub category: String,
    pub default: String,
    #[serde(default, alias = "default_secondary")]
    pub secondary: Option<String>,
    #[serde(default)]
    pub modifiers: Vec<String>,
    #[serde(default)]
    pub tab: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub on_trigger: Option<String>,
}

fn default_keybind_category() -> String {
    "General".to_string()
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModConfigSchema {
    #[serde(alias = "id")]
    pub mod_id: Option<String>,
    pub name: String,
    pub options: Vec<ConfigOptionDef>,
    pub modes: Vec<ConfigModeDef>,
    pub pages: Vec<ConfigPageDef>,
    pub keybinds: Vec<KeybindDef>,
}

#[derive(Deserialize)]
struct TomlOptionDef {
    id: String,
    label: String,
    #[serde(default, rename = "type", alias = "option_type")]
    option_type: ConfigOptionType,
    #[serde(default)]
    default: Option<ConfigValue>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    options: Vec<ConfigValue>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    min: Option<f64>,
    #[serde(default)]
    max: Option<f64>,
    #[serde(default)]
    step: Option<f64>,
    #[serde(default)]
    show_if: Option<ShowIfRule>,
    #[serde(default)]
    button_text: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    on_click: Option<String>,
}

impl From<TomlOptionDef> for ConfigOptionDef {
    fn from(t: TomlOptionDef) -> Self {
        Self {
            id: t.id,
            label: t.label,
            option_type: t.option_type,
            default: t.default,
            description: t.description,
            options: t.options,
            labels: t.labels,
            min: t.min,
            max: t.max,
            step: t.step,
            show_if: t.show_if,
            button_text: t.button_text,
            aliases: t.aliases,
            on_click: t.on_click,
        }
    }
}

#[derive(Deserialize)]
struct TomlPageDef {
    id: String,
    title: String,
    #[serde(rename = "type", alias = "page_type", default)]
    page_type: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    action_label: Option<String>,
    #[serde(default)]
    action_on_click: Option<String>,
}

impl From<TomlPageDef> for ConfigPageDef {
    fn from(t: TomlPageDef) -> Self {
        Self {
            id: t.id,
            title: t.title,
            page_type: t.page_type,
            content: t.content,
            action_label: t.action_label,
            action_on_click: t.action_on_click,
        }
    }
}

#[derive(Deserialize)]
struct TomlKeybindDef {
    id: String,
    name: String,
    #[serde(default)]
    category: Option<String>,
    default: String,
    #[serde(default, alias = "default_secondary")]
    secondary: Option<String>,
    #[serde(default)]
    modifiers: Vec<String>,
    #[serde(default)]
    tab: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    on_trigger: Option<String>,
}

impl From<TomlKeybindDef> for KeybindDef {
    fn from(t: TomlKeybindDef) -> Self {
        Self {
            id: t.id,
            name: t.name,
            category: t.category.unwrap_or_else(default_keybind_category),
            default: t.default,
            secondary: t.secondary,
            modifiers: t.modifiers,
            tab: t.tab,
            description: t.description,
            on_trigger: t.on_trigger,
        }
    }
}

#[derive(Deserialize)]
struct TomlModConfig {
    #[serde(alias = "id", default)]
    mod_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    options: Vec<TomlOptionDef>,
    #[serde(default)]
    modes: Vec<ConfigModeDef>,
    #[serde(default)]
    pages: Vec<TomlPageDef>,
    #[serde(default)]
    keybinds: Vec<TomlKeybindDef>,
}

impl From<TomlModConfig> for ModConfigSchema {
    fn from(t: TomlModConfig) -> Self {
        Self {
            mod_id: t.mod_id,
            name: t.name.unwrap_or_else(|| "Mod".to_string()),
            options: t.options.into_iter().map(Into::into).collect(),
            modes: t.modes,
            pages: t.pages.into_iter().map(Into::into).collect(),
            keybinds: t.keybinds.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Deserialize)]
struct ConfigEnvelope {
    config: TomlModConfig,
}

impl ModConfigSchema {
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        if path.extension().is_some_and(|ext| ext == "toml") {
            Self::from_toml_str(&raw)
        } else {
            Self::from_lua_str(&raw)
        }
    }

    pub fn from_toml_str(raw: &str) -> Result<Self, String> {
        if let Ok(env) = toml::from_str::<ConfigEnvelope>(raw) {
            return Ok(env.config.into());
        }
        toml::from_str::<TomlModConfig>(raw)
            .map(Into::into)
            .map_err(|e| format!("Failed to parse config TOML: {e}"))
    }

    pub fn from_mod_dir(dir: &Path) -> Result<Option<Self>, String> {
        let config_toml = dir.join("config.toml");
        if config_toml.is_file() {
            let content = std::fs::read_to_string(&config_toml)
                .map_err(|e| format!("Failed to read {}: {e}", config_toml.display()))?;
            return Self::from_toml_str(&content).map(Some);
        }

        let qm_toml = dir.join("qm.toml");
        if qm_toml.is_file() {
            if let Ok(content) = std::fs::read_to_string(&qm_toml) {
                if let Ok(env) = toml::from_str::<ConfigEnvelope>(&content) {
                    return Ok(Some(env.config.into()));
                }
            }
        }

        let config_lua = dir.join("config.lua");
        if config_lua.is_file() {
            let content = std::fs::read_to_string(&config_lua)
                .map_err(|e| format!("Failed to read {}: {e}", config_lua.display()))?;
            return Self::from_lua_str(&content).map(Some);
        }

        // Check immediate subdirectories for versioned layouts (e.g. 1.0.0/)
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut subdirs = Vec::new();
            for entry in entries.filter_map(|e| e.ok()) {
                let p = entry.path();
                if p.is_dir() {
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                    if !name.starts_with('.') && name != "qm" && name != "assets" {
                        subdirs.push(p);
                    }
                }
            }
            subdirs.sort();
            for sub in subdirs.into_iter().rev() {
                let sub_cfg = sub.join("config.toml");
                if sub_cfg.is_file() {
                    let content = std::fs::read_to_string(&sub_cfg)
                        .map_err(|e| format!("Failed to read {}: {e}", sub_cfg.display()))?;
                    return Self::from_toml_str(&content).map(Some);
                }
                let sub_qm = sub.join("qm.toml");
                if sub_qm.is_file() {
                    if let Ok(content) = std::fs::read_to_string(&sub_qm) {
                        if let Ok(env) = toml::from_str::<ConfigEnvelope>(&content) {
                            return Ok(Some(env.config.into()));
                        }
                    }
                }
                let sub_lua = sub.join("config.lua");
                if sub_lua.is_file() {
                    let content = std::fs::read_to_string(&sub_lua)
                        .map_err(|e| format!("Failed to read {}: {e}", sub_lua.display()))?;
                    return Self::from_lua_str(&content).map(Some);
                }
            }
        }

        Ok(None)
    }

    /// Parses a `config.lua` file without executing Lua code.
    /// Extracts `name`, `id`, and the `options = { ... }` table, including declarative `showIf` tables
    /// and local helper functions like `isHalloweenActive`.
    pub fn from_lua_str(raw: &str) -> Result<Self, String> {
        // 1. Detect local helper predicates like:
        //    local function isHalloweenActive() return ... == "Halloween" end
        let mut helper_rules: HashMap<String, ShowIfRule> = HashMap::new();
        let fn_re = regex_lite::Regex::new(
            r#"local\s+function\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(\s*\)\s*([\s\S]*?)end"#,
        )
        .unwrap();
        let eq_re = regex_lite::Regex::new(r#"==\s*"([^"]+)""#).unwrap();

        for cap in fn_re.captures_iter(raw) {
            let fn_name = &cap[1];
            let body = &cap[2];
            if let Some(eq_cap) = eq_re.captures(body) {
                let target_val = eq_cap[1].to_string();
                helper_rules.insert(
                    fn_name.to_string(),
                    ShowIfRule {
                        option_id: "event_select".to_string(),
                        equals: vec![ConfigValue::String(target_val)],
                    },
                );
            }
        }

        // 2. Strip `--` line comments (preserving strings) so brace matching is clean
        let stripped = strip_lua_comments(raw);

        let name = extract_string_field(&stripped, "name").unwrap_or_else(|| "Mod".to_string());
        let mod_id = extract_string_field(&stripped, "id");

        let mut options = Vec::new();
        if let Some(options_block) = extract_table_field(&stripped, "options") {
            for entry_block in split_top_level_tables(&options_block) {
                if let Some(opt) = parse_option_entry(&entry_block, &helper_rules) {
                    options.push(opt);
                }
            }
        }

        Ok(ModConfigSchema {
            mod_id,
            name,
            options,
            modes: Vec::new(),
            pages: Vec::new(),
            keybinds: Vec::new(),
        })
    }
}

fn strip_lua_comments(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for line in raw.lines() {
        let mut in_str: Option<char> = None;
        let mut prev_backslash = false;
        let chars: Vec<char> = line.chars().collect();
        let mut idx = 0;
        while idx < chars.len() {
            let c = chars[idx];
            if let Some(q) = in_str {
                out.push(c);
                if c == q && !prev_backslash {
                    in_str = None;
                }
                prev_backslash = c == '\\' && !prev_backslash;
                idx += 1;
            } else if c == '"' || c == '\'' {
                in_str = Some(c);
                out.push(c);
                prev_backslash = false;
                idx += 1;
            } else if c == '-' && idx + 1 < chars.len() && chars[idx + 1] == '-' {
                break;
            } else {
                out.push(c);
                idx += 1;
            }
        }
        out.push('\n');
    }
    out
}

fn extract_string_field(block: &str, key: &str) -> Option<String> {
    let pattern = format!(r#"\b{}\s*=\s*"([^"]*)""#, regex_lite::escape(key));
    let re = regex_lite::Regex::new(&pattern).ok()?;
    re.captures(block).map(|c| c[1].to_string())
}

fn extract_number_field(block: &str, key: &str) -> Option<f64> {
    let pattern = format!(
        r#"\b{}\s*=\s*(-?[0-9]+(?:\.[0-9]+)?)"#,
        regex_lite::escape(key)
    );
    let re = regex_lite::Regex::new(&pattern).ok()?;
    re.captures(block).and_then(|c| c[1].parse::<f64>().ok())
}

fn extract_scalar_field(block: &str, key: &str) -> Option<ConfigValue> {
    if let Some(s) = extract_string_field(block, key) {
        return Some(ConfigValue::String(s));
    }
    if let Some(n) = extract_number_field(block, key) {
        return Some(ConfigValue::Number(n));
    }
    let bool_pat = format!(r#"\b{}\s*=\s*(true|false)\b"#, regex_lite::escape(key));
    if let Ok(re) = regex_lite::Regex::new(&bool_pat) {
        if let Some(cap) = re.captures(block) {
            return Some(ConfigValue::Bool(&cap[1] == "true"));
        }
    }
    None
}

/// Finds `key = { ... }` and returns the content inside the balanced `{ ... }`.
fn extract_table_field(block: &str, key: &str) -> Option<String> {
    let pattern = format!(r#"\b{}\s*=\s*\{{"#, regex_lite::escape(key));
    let re = regex_lite::Regex::new(&pattern).ok()?;
    let m = re.find(block)?;
    let open_brace_pos = m.end() - 1;
    extract_balanced_braces(&block[open_brace_pos..])
}

fn extract_balanced_braces(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_str: Option<u8> = None;
    let mut prev_escape = false;

    for (i, &b) in bytes.iter().enumerate() {
        if let Some(q) = in_str {
            if b == q && !prev_escape {
                in_str = None;
            }
            prev_escape = b == b'\\' && !prev_escape;
        } else if b == b'"' || b == b'\'' {
            in_str = Some(b);
            prev_escape = false;
        } else if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(s[1..i].to_string());
            }
        }
    }
    None
}

/// Splits the interior of `options = { { ... }, { ... } }` into top-level `{ ... }` option blocks,
/// ignoring braces inside `function() ... end` bodies.
fn split_top_level_tables(inner: &str) -> Vec<String> {
    let mut tables = Vec::new();
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(content) = extract_balanced_braces(&inner[i..]) {
                let consumed = content.len() + 2;
                tables.push(content);
                i += consumed;
                continue;
            }
        }
        i += 1;
    }
    tables
}

fn parse_option_entry(
    entry: &str,
    helper_rules: &HashMap<String, ShowIfRule>,
) -> Option<ConfigOptionDef> {
    // Strip out `get = function(...) ... end` and `set = function(...) ... end` before field extraction
    let fn_body_re =
        regex_lite::Regex::new(r#"\b(get|set)\s*=\s*function\b[\s\S]*?\bend\b"#).unwrap();
    let clean = fn_body_re.replace_all(entry, "").into_owned();

    let id = extract_string_field(&clean, "id")?;
    let label = extract_string_field(&clean, "label")
        .or_else(|| extract_string_field(&clean, "name"))
        .unwrap_or_else(|| id.clone());
    let raw_type = extract_string_field(&clean, "type").unwrap_or_else(|| "checkbox".to_string());
    let option_type = match raw_type.to_lowercase().as_str() {
        "checkbox" | "toggle" | "bool" | "boolean" => ConfigOptionType::Checkbox,
        "cycle" | "choice" | "select" | "dropdown" => ConfigOptionType::Cycle,
        "slider" | "range" | "number" => ConfigOptionType::Slider,
        "separator" | "header" => ConfigOptionType::Separator,
        "button" | "action" => ConfigOptionType::Button,
        _ => ConfigOptionType::Checkbox,
    };

    let default = extract_scalar_field(&clean, "default");
    let description = extract_string_field(&clean, "description")
        .or_else(|| extract_string_field(&clean, "tooltip"));
    let min = extract_number_field(&clean, "min");
    let max = extract_number_field(&clean, "max");
    let step = extract_number_field(&clean, "step");

    let options = extract_table_field(&clean, "options")
        .map(|s| parse_scalar_array(&s))
        .unwrap_or_default();
    let labels = extract_table_field(&clean, "labels")
        .map(|s| {
            parse_scalar_array(&s)
                .into_iter()
                .map(|v| v.as_display_string())
                .collect()
        })
        .unwrap_or_default();

    // Parse `showIf`: either declarative table `showIf = { id = "event_select", value = "Halloween" }`
    // or function identifier `showIf = isHalloweenActive`
    let mut show_if = None;
    if let Some(show_tbl) = extract_table_field(&clean, "showIf") {
        if let Some(dep_id) = extract_string_field(&show_tbl, "id") {
            let mut equals = Vec::new();
            if let Some(v) = extract_scalar_field(&show_tbl, "value") {
                equals.push(v);
            }
            if let Some(vals_tbl) = extract_table_field(&show_tbl, "values") {
                equals.extend(parse_scalar_array(&vals_tbl));
            }
            if !equals.is_empty() {
                show_if = Some(ShowIfRule {
                    option_id: dep_id,
                    equals,
                });
            }
        }
    } else {
        let ident_re = regex_lite::Regex::new(r#"\bshowIf\s*=\s*([A-Za-z_][A-Za-z0-9_]*)"#).ok()?;
        if let Some(cap) = ident_re.captures(&clean) {
            if let Some(rule) = helper_rules.get(&cap[1]) {
                show_if = Some(rule.clone());
            }
        }
    }

    Some(ConfigOptionDef {
        id,
        label,
        option_type,
        default,
        description,
        options,
        labels,
        min,
        max,
        step,
        show_if,
        button_text: extract_string_field(&clean, "button_text")
            .or_else(|| extract_string_field(&clean, "buttonText")),
        aliases: Vec::new(),
        on_click: extract_string_field(&clean, "on_click")
            .or_else(|| extract_string_field(&clean, "onClick")),
    })
}

fn parse_scalar_array(raw: &str) -> Vec<ConfigValue> {
    let mut out = Vec::new();
    let bytes = raw.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            if let Ok(s) = std::str::from_utf8(&bytes[start..i]) {
                out.push(ConfigValue::String(s.to_string()));
            }
            if i < bytes.len() {
                i += 1;
            }
        } else {
            let start = i;
            while i < bytes.len() && bytes[i] != b',' && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if let Ok(tok) = std::str::from_utf8(&bytes[start..i]) {
                if tok == "true" {
                    out.push(ConfigValue::Bool(true));
                } else if tok == "false" {
                    out.push(ConfigValue::Bool(false));
                } else if let Ok(n) = tok.parse::<f64>() {
                    out.push(ConfigValue::Number(n));
                }
            }
        }
    }
    out
}

/// Persistent per-profile mod settings table (`mod_id -> (option_id -> ConfigValue)`),
/// compatible with `saveModSettings.lua` read/written by `GMenu` in-game.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModSettingsStore {
    pub mods: HashMap<String, HashMap<String, ConfigValue>>,
}

impl ModSettingsStore {
    pub fn from_path(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        Self::from_lua_str(&raw)
    }

    pub fn from_lua_str(raw: &str) -> Result<Self, String> {
        let doc = SerDocument::parse(raw)?;
        let mut store = ModSettingsStore::default();

        for (mod_id, val) in &doc.root.map {
            if let Some(mod_tbl) = doc.resolve_table(val) {
                let mut opts = HashMap::new();
                for (opt_id, opt_val) in &mod_tbl.map {
                    match opt_val {
                        SerValue::Bool(b) => {
                            opts.insert(opt_id.clone(), ConfigValue::Bool(*b));
                        }
                        SerValue::Number(n) => {
                            opts.insert(opt_id.clone(), ConfigValue::Number(*n));
                        }
                        SerValue::String(s) => {
                            opts.insert(opt_id.clone(), ConfigValue::String(s.clone()));
                        }
                        _ => {}
                    }
                }
                store.mods.insert(mod_id.clone(), opts);
            }
        }

        Ok(store)
    }

    /// Serializes the settings store in valid `ser.lua` format so both `GMenu`'s `love.filesystem.load`
    /// and `SerDocument::parse` read it identically.
    pub fn to_ser_lua_string(&self) -> String {
        if self.mods.is_empty() {
            return "return {}".to_string();
        }

        let mut mod_ids: Vec<&String> = self.mods.keys().collect();
        mod_ids.sort();

        let mut lines = Vec::new();
        lines.push("local _={}".to_string());

        let mut root_pairs = Vec::new();
        for (idx, mod_id) in mod_ids.iter().enumerate() {
            let memo_idx = idx + 1;
            let opts = &self.mods[*mod_id];
            let mut opt_keys: Vec<&String> = opts.keys().collect();
            opt_keys.sort();

            let mut kv_strs = Vec::new();
            for k in opt_keys {
                let v_str = match &opts[k] {
                    ConfigValue::Bool(b) => b.to_string(),
                    ConfigValue::Number(n) => format!("{n}"),
                    ConfigValue::String(s) => format!("{:?}", s),
                };
                kv_strs.push(format!("{}={}", format_lua_key(k), v_str));
            }
            lines.push(format!("_[{}]={{{}}}", memo_idx, kv_strs.join(",")));
            root_pairs.push(format!("{}=_[{}]", format_lua_key(mod_id), memo_idx));
        }

        lines.push(format!("return {{{}}}", root_pairs.join(",")));
        lines.join("\n")
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory {}: {e}", parent.display()))?;
        }
        std::fs::write(path, self.to_ser_lua_string())
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))
    }

    /// Resets all options for `mod_id` back to the defaults declared in `schema`.
    pub fn reset_mod_to_defaults(&mut self, mod_id: &str, schema: &ModConfigSchema) {
        let mut defaults = HashMap::new();
        for opt in &schema.options {
            if let Some(ref def_val) = opt.default {
                defaults.insert(opt.id.clone(), def_val.clone());
            }
        }
        self.mods.insert(mod_id.to_string(), defaults);
    }

    /// Safe-reset: preserves valid user choices for `mod_id` and resets only unknown or out-of-range keys to defaults.
    pub fn safe_reset_mod(&mut self, mod_id: &str, schema: &ModConfigSchema) {
        let entry = self.mods.entry(mod_id.to_string()).or_default();
        let valid_ids: std::collections::HashSet<String> =
            schema.options.iter().map(|o| o.id.clone()).collect();
        entry.retain(|k, _| valid_ids.contains(k));

        for opt in &schema.options {
            let mut needs_reset = false;
            if let Some(cur) = entry.get(&opt.id) {
                match opt.option_type {
                    ConfigOptionType::Slider => {
                        if let ConfigValue::Number(n) = cur {
                            if let Some(min) = opt.min {
                                if *n < min {
                                    needs_reset = true;
                                }
                            }
                            if let Some(max) = opt.max {
                                if *n > max {
                                    needs_reset = true;
                                }
                            }
                        } else {
                            needs_reset = true;
                        }
                    }
                    ConfigOptionType::Cycle
                        if !opt.options.is_empty() && !opt.options.contains(cur) =>
                    {
                        needs_reset = true;
                    }
                    _ => {}
                }
            } else {
                needs_reset = true;
            }
            if needs_reset {
                if let Some(ref def_val) = opt.default {
                    entry.insert(opt.id.clone(), def_val.clone());
                }
            }
        }
    }
}

fn format_lua_key(k: &str) -> String {
    let is_ident = !k.is_empty()
        && k.chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_')
            .unwrap_or(false)
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if is_ident {
        k.to_string()
    } else {
        format!("[{:?}]", k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gspecialevents_and_tactical_overhaul_configs() {
        let to_cfg = r#"
local config = {
  name = "Tactical Overhaul",
  options = {
    {
      id = "squad_limit",
      label = "Squad Size Limit",
      type = "cycle",
      options = { 6, 8, 10, 12, 16, 24, 32, "OFF" },
      labels = { "6", "8", "10", "12", "16", "24", "32", "OFF" },
      default = 6,
      description = "Exposed soldier threshold",
      get = function() return 6 end,
      set = function(val) end
    },
    {
      id = "smart_move",
      label = "Smart Squad Move",
      type = "checkbox",
      default = 1,
      description = "Weighted cover distribution"
    }
  }
}
return config"#;

        let schema = ModConfigSchema::from_lua_str(to_cfg).unwrap();
        assert_eq!(schema.name, "Tactical Overhaul");
        assert_eq!(schema.options.len(), 2);
        assert_eq!(schema.options[0].id, "squad_limit");
        assert_eq!(schema.options[0].option_type, ConfigOptionType::Cycle);
        assert_eq!(schema.options[0].options.len(), 8);
        assert_eq!(schema.options[1].id, "smart_move");
        assert_eq!(schema.options[1].option_type, ConfigOptionType::Checkbox);
    }

    #[test]
    fn roundtrips_save_mod_settings_lua_including_legacy_broken_prefix() {
        let broken = "return local _={}\n_[2]={safe_destination=1,pathfinding=1}\n_[1]={}\nreturn {GDumper=_[1],GCore=_[2]}";
        let store = ModSettingsStore::from_lua_str(broken).unwrap();
        assert_eq!(
            store.mods["GCore"]["safe_destination"],
            ConfigValue::Number(1.0)
        );

        let serialized = store.to_ser_lua_string();
        assert!(serialized.starts_with("local _={}\n"));
        let store2 = ModSettingsStore::from_lua_str(&serialized).unwrap();
        assert_eq!(store, store2);
    }
}

#[cfg(test)]
#[path = "config_schema_tests.rs"]
mod config_schema_tests;
