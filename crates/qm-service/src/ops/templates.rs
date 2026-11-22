//! Mod templates in the Mod Engine (Stage D): the catalog, a preview of a template's files, a
//! new workspace mod made from one, and an example added to a mod being made.
//!
//! Adding an example never overwrites: new files are copied in, patch, option and keybind
//! tables are appended to the mod's own `qm.toml` and `config.toml` (their comments and
//! layout kept), and a feature the example needs is listed in `mod.json`. Anything that would
//! clash (a file, a patch id, a module name, an option id) blocks the whole addition, and the
//! preview says what.
use crate::error::{ErrorKind, QmError, QmResult};
use crate::events::Scope;
use crate::ops::engine::{decode_text, encode_text, write_atomic};
use crate::ops::studio::{mod_id_of, with_feature};
use crate::service::Service;
use qm_manager::templates::{self, TemplateInfo, TemplateNames};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use toml_edit::{ArrayOfTables, Document as DocumentMut, Item, Table};

/// A template file as the catalog shows it, with the template's own names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TemplateFileView {
    pub path: String,
    /// The text; None for an image or a sound.
    pub text: Option<String>,
    pub size_bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum TemplateChangeKind {
    /// A new file.
    Add,
    /// Tables added to a file the mod has.
    Merge,
    /// Left out: the mod has its own.
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TemplateChange {
    pub path: String,
    pub kind: TemplateChangeKind,
    pub detail: String,
}

/// What adding an example to a mod would change, or why it cannot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TemplateAddPreview {
    pub changes: Vec<TemplateChange>,
    /// Clashes that stop the addition; empty when it can go ahead.
    pub blocked: Vec<String>,
}

/// The tables of a patch file and of `config.toml` that hold lists of entries.
const PATCH_LISTS: [&str; 7] = [
    "patches",
    "pattern",
    "regex",
    "copy",
    "module",
    "overrides",
    "asset_choices",
];
const CONFIG_LISTS: [&str; 4] = ["options", "keybinds", "modes", "pages"];

impl Service {
    /// Every template, by wiki part.
    pub fn templates_list(&self) -> Vec<TemplateInfo> {
        templates::templates()
    }

    /// A template's files with its own names, for the catalog's preview.
    pub fn template_preview(&self, id: &str) -> QmResult<Vec<TemplateFileView>> {
        let files = templates::render(id, &TemplateNames::example()).map_err(not_found)?;
        Ok(files
            .into_iter()
            .map(|(path, bytes)| TemplateFileView {
                size_bytes: u32::try_from(bytes.len()).unwrap_or(u32::MAX),
                text: (!bytes.contains(&0))
                    .then(|| String::from_utf8(bytes).ok())
                    .flatten(),
                path,
            })
            .collect())
    }

    /// A new workspace mod from a template. Returns its folder name.
    pub fn template_create(&self, id: &str, name: &str, mod_id: &str) -> QmResult<String> {
        self.ensure_no_install()?;
        let ws = self.engine_workspace()?;
        let names = TemplateNames::new(mod_id, name, "").map_err(invalid)?;
        if ws.join(&names.folder).exists() {
            return Err(QmError::new(
                ErrorKind::Conflict,
                format!("The workspace already has a folder named {}.", names.folder),
            ));
        }
        templates::create(&ws, id, &names).map_err(|e| QmError::new(ErrorKind::Io, e))?;
        if self.paths().workspace_mode {
            self.invalidate(Scope::Mods);
        }
        Ok(names.folder)
    }

    /// What adding the template's example to the workspace mod in `folder` would change.
    pub fn template_add_preview(&self, folder: &str, id: &str) -> QmResult<TemplateAddPreview> {
        let dir = self.engine_project_dir(folder)?;
        Ok(plan_add(&dir, folder, id)?.0)
    }

    /// Adds the template's example to the workspace mod in `folder`. Returns the files changed.
    pub fn template_add(&self, folder: &str, id: &str) -> QmResult<Vec<String>> {
        self.ensure_no_install()?;
        let dir = self.engine_project_dir(folder)?;
        let (preview, writes) = plan_add(&dir, folder, id)?;
        if let Some(why) = preview.blocked.first() {
            return Err(QmError::new(ErrorKind::Conflict, why.clone()));
        }
        let mut changed = Vec::new();
        for (rel, bytes) in writes {
            qm_manager::library::validate_safe_relative_path(&rel).map_err(invalid)?;
            let path = dir.join(&rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| QmError::new(ErrorKind::Io, e.to_string()))?;
            }
            write_atomic(&path, &bytes)?;
            changed.push(rel);
        }
        Ok(changed)
    }
}

fn invalid(msg: impl Into<String>) -> QmError {
    QmError::new(ErrorKind::InvalidInput, msg)
}

fn not_found(msg: impl Into<String>) -> QmError {
    QmError::new(ErrorKind::NotFound, msg)
}

/// The mod's display name: `mod.json`'s, else the folder's.
fn mod_name_of(dir: &Path, folder: &str) -> String {
    std::fs::read_to_string(dir.join("mod.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| folder.to_string())
}

/// A file of the mod as text, keeping its line endings and BOM for writing back.
fn read_text(dir: &Path, rel: &str) -> QmResult<Option<(String, bool, bool)>> {
    match std::fs::read(dir.join(rel)) {
        Ok(bytes) => {
            let f = decode_text(rel, &bytes)?;
            Ok(Some((f.text, f.crlf, f.bom)))
        }
        Err(_) => Ok(None),
    }
}

fn parse(rel: &str, text: &str) -> QmResult<DocumentMut> {
    text.parse::<DocumentMut>().map_err(|e| {
        invalid(format!(
            "{rel} has a mistake, so the example cannot be added to it: {}",
            e.message()
        ))
    })
}

/// The id of an entry of a list table: its own `id`, or for a `[[patches]]` entry, the id in
/// its type table.
fn entry_id(t: &Table) -> Option<String> {
    if let Some(id) = t.get("id").and_then(Item::as_str) {
        return Some(id.to_string());
    }
    ["pattern", "regex", "copy", "module"].iter().find_map(|k| {
        t.get(k)
            .and_then(Item::as_table)?
            .get("id")
            .and_then(Item::as_str)
            .map(str::to_string)
    })
}

/// A module patch's `name` (the name the game requires it by), when the entry is one.
fn module_name(key: &str, t: &Table) -> Option<String> {
    let table = if key == "module" {
        Some(t)
    } else {
        t.get("module").and_then(Item::as_table)
    };
    table?
        .get("name")
        .and_then(Item::as_str)
        .map(str::to_string)
}

/// The highest document position of any table in `t` (tables keep their order by it).
fn max_position(t: &Table) -> usize {
    let mut max = t.position().unwrap_or(0);
    for (_, item) in t.iter() {
        match item {
            Item::Table(sub) => max = max.max(max_position(sub)),
            Item::ArrayOfTables(list) => {
                for sub in list.iter() {
                    max = max.max(max_position(sub));
                }
            }
            _ => {}
        }
    }
    max
}

/// Gives `t` and the tables in it positions from `next` on, so a table copied from another
/// file lands after the mod's own instead of among them.
fn renumber(t: &mut Table, next: &mut usize) {
    t.set_position(*next);
    *next += 1;
    for (_, item) in t.iter_mut() {
        match item {
            Item::Table(sub) => renumber(sub, next),
            Item::ArrayOfTables(list) => {
                for sub in list.iter_mut() {
                    renumber(sub, next);
                }
            }
            _ => {}
        }
    }
}

fn ids_in(doc: &DocumentMut, lists: &[&str]) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut ids = BTreeSet::new();
    let mut modules = BTreeSet::new();
    for key in lists {
        if let Some(list) = doc.get(key).and_then(Item::as_array_of_tables) {
            for t in list.iter() {
                ids.extend(entry_id(t));
                modules.extend(module_name(key, t));
            }
        }
    }
    (ids, modules)
}

/// Appends the example's list entries and plain tables to the mod's document. Returns what was
/// added, and the clashes.
fn merge_doc(
    mine: &mut DocumentMut,
    example: &DocumentMut,
    rel: &str,
    lists: &[&str],
    template: &TemplateInfo,
) -> (Vec<String>, Vec<String>) {
    let (ids, modules) = ids_in(mine, lists);
    let mut next = max_position(mine.as_table()) + 1;
    let mut added = Vec::new();
    let mut clashes = Vec::new();
    let mut first = true;
    for key in lists {
        let Some(list) = example.get(key).and_then(Item::as_array_of_tables) else {
            continue;
        };
        for t in list.iter() {
            if let Some(id) = entry_id(t).filter(|id| ids.contains(id)) {
                clashes.push(format!("{rel} already has {id}."));
                continue;
            }
            if let Some(m) = module_name(key, t).filter(|m| modules.contains(m)) {
                clashes.push(format!("{rel} already loads a module named {m}."));
                continue;
            }
            let mut entry = t.clone();
            let prefix = if first {
                format!("\n# Added from the template \"{}\".\n", template.name)
            } else {
                "\n".to_string()
            };
            first = false;
            entry.decor_mut().set_prefix(prefix);
            renumber(&mut entry, &mut next);
            if !mine.contains_key(key) {
                mine.insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
            }
            if let Some(target) = mine.get_mut(key).and_then(Item::as_array_of_tables_mut) {
                target.push(entry);
                added.push(entry_id(t).unwrap_or_else(|| format!("a [[{key}]] entry")));
            }
        }
    }
    // Plain tables whose keys are merged one by one ([vars], [sprites]); [publish] whole.
    for key in ["vars", "sprites"] {
        let Some(table) = example.get(key).and_then(Item::as_table) else {
            continue;
        };
        if !mine.contains_key(key) {
            let mut fresh = Table::new();
            fresh.set_position(next);
            next += 1;
            mine.insert(key, Item::Table(fresh));
        }
        if let Some(target) = mine.get_mut(key).and_then(Item::as_table_mut) {
            for (k, v) in table.iter() {
                if target.contains_key(k) {
                    clashes.push(format!("{rel} already has [{key}] {k}."));
                } else {
                    target.insert(k, v.clone());
                    added.push(format!("[{key}] {k}"));
                }
            }
        }
    }
    if let Some(publish) = example.get("publish") {
        if !mine.contains_key("publish") {
            let mut publish = publish.clone();
            if let Some(t) = publish.as_table_mut() {
                renumber(t, &mut next);
            }
            mine.insert("publish", publish);
            added.push("[publish]".to_string());
        }
    }
    (added, clashes)
}

/// What a `[[weapons]]` or `[[vehicles]]` entry of a faction stands for (the game's thing it
/// reskins, or its own tag), or a `[[ordnance]]` or `[[explosions]]` entry's id.
fn faction_entry_key(t: &Table) -> Option<String> {
    ["reskin", "tag", "id"].iter().find_map(|k| {
        t.get(k)
            .and_then(Item::as_str)
            .map(|v| format!("{k} = \"{v}\""))
    })
}

/// The state of one merge into `faction.toml`.
struct FactionMerge<'a> {
    template: &'a str,
    next: usize,
    commented: bool,
    added: Vec<String>,
    clashes: Vec<String>,
}

impl FactionMerge<'_> {
    /// The comment above the first thing added, a blank line above the others.
    fn prefix(&mut self) -> String {
        if std::mem::replace(&mut self.commented, true) {
            "\n".to_string()
        } else {
            format!("\n# Added from the template \"{}\".\n", self.template)
        }
    }

    /// Adds the example's table `key` (at `path`) to `parent`: whole when the mod has none,
    /// else key by key, a key both have being a clash.
    fn table(&mut self, parent: &mut Table, key: &str, example: &Table, path: &str) {
        match parent.get_mut(key) {
            None => {
                let mut t = example.clone();
                let prefix = self.prefix();
                set_first_prefix(&mut t, prefix);
                renumber(&mut t, &mut self.next);
                parent.insert(key, Item::Table(t));
                self.added.push(format!("[{path}]"));
            }
            Some(Item::Table(mine)) => {
                for (k, v) in example.iter() {
                    let sub = format!("{path}.{k}");
                    match v {
                        Item::Table(t) if mine.get(k).is_none_or(Item::is_table) => {
                            self.table(mine, k, t, &sub);
                        }
                        _ if mine.get(k).is_some_and(|m| same_value(m, v)) => {}
                        _ if mine.contains_key(k) => {
                            self.clashes
                                .push(format!("faction.toml already has {sub}."));
                        }
                        _ => {
                            mine.insert(k, v.clone());
                            self.added.push(sub);
                        }
                    }
                }
            }
            Some(_) => self
                .clashes
                .push(format!("faction.toml already has {path}.")),
        }
    }
}

/// Whether two values are the same, whatever their spacing and comments.
fn same_value(a: &Item, b: &Item) -> bool {
    // Read back as plain TOML data, which has no spacing or comments to differ in.
    let norm = |i: &Item| {
        let mut v = i.as_value()?.clone();
        v.decor_mut().clear();
        toml::from_str::<toml::Table>(&format!("v = {v}")).ok()
    };
    norm(a).is_some_and(|x| Some(x) == norm(b))
}

/// Gives `prefix` to the first table of `t` that is written with a header (an implicit
/// `[units]` is not; its `[units.base]` is).
fn set_first_prefix(t: &mut Table, prefix: String) {
    if !t.is_implicit() {
        t.decor_mut().set_prefix(prefix);
        return;
    }
    if let Some(sub) = t.iter_mut().find_map(|(_, item)| item.as_table_mut()) {
        set_first_prefix(sub, prefix);
    }
}

/// Merges an example's faction sections into the mod's `faction.toml`. The faction's own keys
/// (`id`, `name`, `replaces`, ...) stay the mod's; tables and keys the mod lacks are added, and
/// a key it has with the same value is left; `[[weapons]]` and `[[vehicles]]` entries are
/// appended, or fill in the mod's entry that reskins or tags the same thing. A key the mod sets
/// to something else is a clash. Returns what was added, and the clashes.
fn merge_faction(
    mine: &mut DocumentMut,
    example: &DocumentMut,
    template: &TemplateInfo,
) -> (Vec<String>, Vec<String>) {
    let mut m = FactionMerge {
        template: &template.name,
        next: max_position(mine.as_table()) + 1,
        commented: false,
        added: Vec::new(),
        clashes: Vec::new(),
    };
    for (key, item) in example.iter() {
        match item {
            Item::ArrayOfTables(list) => {
                for t in list.iter() {
                    let id = faction_entry_key(t);
                    // An entry for the same thing: the keys it lacks are added to it, the same
                    // values are already there, a different value is a clash.
                    let same = id.as_ref().and_then(|id| {
                        mine.get_mut(key)
                            .and_then(Item::as_array_of_tables_mut)?
                            .iter_mut()
                            .find(|e| faction_entry_key(e).as_ref() == Some(id))
                    });
                    if let (Some(existing), Some(id)) = (same, id.as_ref()) {
                        let mut keys = Vec::new();
                        for (k, v) in t.iter() {
                            match existing.get(k) {
                                None => {
                                    existing.insert(k, v.clone());
                                    keys.push(k.to_string());
                                }
                                Some(e) if same_value(e, v) => {}
                                Some(_) => m.clashes.push(format!(
                                    "faction.toml's [[{key}]] entry with {id} already sets {k}."
                                )),
                            }
                        }
                        if !keys.is_empty() {
                            m.added.push(format!("[[{key}]] {id}: {}", keys.join(", ")));
                        }
                        continue;
                    }
                    let mut entry = t.clone();
                    let prefix = m.prefix();
                    entry.decor_mut().set_prefix(prefix);
                    renumber(&mut entry, &mut m.next);
                    if !mine.contains_key(key) {
                        mine.insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
                    }
                    if let Some(target) = mine.get_mut(key).and_then(Item::as_array_of_tables_mut) {
                        target.push(entry);
                        m.added.push(match id {
                            Some(id) => format!("[[{key}]] {id}"),
                            None => format!("a [[{key}]] entry"),
                        });
                    }
                }
            }
            Item::Table(t) => m.table(mine.as_table_mut(), key, t, key),
            // The faction's identity stays the mod's.
            _ => {}
        }
    }
    (m.added, m.clashes)
}

type Writes = Vec<(String, Vec<u8>)>;

fn plan_add(dir: &Path, folder: &str, id: &str) -> QmResult<(TemplateAddPreview, Writes)> {
    let template =
        templates::find(id).ok_or_else(|| not_found(format!("There is no template '{id}'.")))?;
    let mut blocked = Vec::new();
    match template.id.as_str() {
        "empty" => blocked.push("The empty template has nothing to add.".to_string()),
        "compat-patch" => blocked
            .push("A compatibility patch is a mod of its own: make it with New mod.".to_string()),
        _ => {}
    }
    let mod_id = mod_id_of(dir, folder);
    let base = TemplateNames::new(&mod_id, &mod_name_of(dir, folder), "")
        .or_else(|_| TemplateNames::new("example_mod", folder, ""))
        .map_err(invalid)?;
    let names = TemplateNames { id: mod_id, ..base };
    let files = templates::render(&template.id, &names).map_err(not_found)?;
    let mut changes = Vec::new();
    let mut writes: Writes = Vec::new();
    for (rel, bytes) in files {
        let exists = dir.join(&rel).exists();
        let text = String::from_utf8(bytes.clone()).ok();
        match rel.as_str() {
            "mod.json" => {
                let wanted: Vec<String> = text
                    .as_deref()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
                    .and_then(|v| v.get("qm_features").cloned())
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|f| f.as_str().map(str::to_string))
                    .collect();
                let Some((mut mine, crlf, bom)) = read_text(dir, "mod.json")? else {
                    continue;
                };
                let mut added = Vec::new();
                for f in wanted {
                    if let Some(edited) = with_feature(&mine, &f) {
                        mine = edited;
                        added.push(f);
                    }
                }
                if !added.is_empty() {
                    changes.push(TemplateChange {
                        path: rel.clone(),
                        kind: TemplateChangeKind::Merge,
                        detail: format!("lists {} in qm_features", added.join(", ")),
                    });
                    writes.push((rel, encode_text(&mine, crlf, bom)));
                }
            }
            "README.md" | "CHANGELOG.md" if exists => changes.push(TemplateChange {
                path: rel,
                kind: TemplateChangeKind::Skip,
                detail: "the mod has its own".to_string(),
            }),
            "qm.toml" | "config.toml" if exists => {
                let lists: &[&str] = if rel == "qm.toml" {
                    &PATCH_LISTS
                } else {
                    &CONFIG_LISTS
                };
                let example = parse(&rel, text.as_deref().unwrap_or(""))?;
                let Some((mine_text, crlf, bom)) = read_text(dir, &rel)? else {
                    continue;
                };
                let mut mine = parse(&rel, &mine_text)?;
                let (added, clashes) = merge_doc(&mut mine, &example, &rel, lists, &template);
                blocked.extend(clashes);
                if !added.is_empty() {
                    changes.push(TemplateChange {
                        path: rel.clone(),
                        kind: TemplateChangeKind::Merge,
                        detail: format!("adds {}", added.join(", ")),
                    });
                    writes.push((rel, encode_text(&mine.to_string(), crlf, bom)));
                }
            }
            "faction.toml" if exists => {
                let example = parse(&rel, text.as_deref().unwrap_or(""))?;
                let Some((mine_text, crlf, bom)) = read_text(dir, &rel)? else {
                    continue;
                };
                let mut mine = parse(&rel, &mine_text)?;
                let (added, clashes) = merge_faction(&mut mine, &example, &template);
                blocked.extend(clashes);
                if !added.is_empty() {
                    changes.push(TemplateChange {
                        path: rel.clone(),
                        kind: TemplateChangeKind::Merge,
                        detail: format!("adds {}", added.join(", ")),
                    });
                    writes.push((rel, encode_text(&mine.to_string(), crlf, bom)));
                }
            }
            _ if exists && std::fs::read(dir.join(&rel)).is_ok_and(|mine| mine == bytes) => {
                changes.push(TemplateChange {
                    path: rel,
                    kind: TemplateChangeKind::Skip,
                    detail: "the mod has the same file".to_string(),
                });
            }
            _ if exists => blocked.push(format!("The mod already has {rel}.")),
            _ => {
                changes.push(TemplateChange {
                    path: rel.clone(),
                    kind: TemplateChangeKind::Add,
                    detail: "a new file".to_string(),
                });
                writes.push((rel, bytes));
            }
        }
    }
    if blocked.is_empty() && writes.is_empty() {
        blocked.push("The mod already has everything this example adds.".to_string());
    }
    Ok((TemplateAddPreview { changes, blocked }, writes))
}
