//! The mod templates: small, commented example mods, one for each ability the modding wiki
//! teaches. They live in `Quartermaster/templates/<id>/` and are embedded at build time, so
//! `qm new` and the launcher always carry the templates of the loader they ship with.
//!
//! A template is written with the wiki's own made-up names (`example_mod`, Example Mod,
//! `ExampleMod`, `exampleSomething` for its global functions, and `example_army` for a
//! faction), so it reads like the wiki and is a mod that loads as it stands. Making a mod from
//! it replaces those names with the new mod's.
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/templates_gen.rs"));

/// The file in a template folder that describes it; it is not written into the mod.
pub const META_FILE: &str = "template.toml";

/// Where the modding wiki is published.
pub const WIKI_URL: &str = "https://gamiseus.github.io/wiki/";

/// The page address of each wiki part on the site (part 1 first): the site gives each
/// `# Part N: Label` heading a page named after its label.
pub const WIKI_PAGES: [&str; 12] = [
    "what-a-mod-is",
    "replacing-art-and-sound",
    "faction-mods",
    "changing-game-code",
    "adding-your-own-lua-files",
    "settings-keybinds-and-menus",
    "working-with-other-mods",
    "game-reference",
    "tools",
    "testing-and-debugging",
    "packaging-and-publishing",
    "the-mod-engine",
];

/// A wiki example a template carries word for word: the `block`-th code block (1-based,
/// counting toml, lua and json blocks) under the heading `section`, found in `file`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct WikiExample {
    pub section: String,
    pub block: u32,
    pub file: String,
}

/// What the catalog shows for a template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    /// The wiki part it belongs to (1 to 12).
    pub part: u32,
    /// Its place within the part.
    pub order: u32,
    /// The wiki heading that explains it, and that heading's link anchor.
    pub section: String,
    pub anchor: String,
    /// The section on the published wiki.
    pub wiki_url: String,
    pub summary: String,
    /// The file to open first.
    pub open: String,
    /// What to try after it works.
    pub try_next: Vec<String>,
    /// Older `qm new` names that pick this template.
    pub aliases: Vec<String>,
    /// Wiki examples this template holds word for word.
    pub examples: Vec<WikiExample>,
    /// Its files, in path order (`template.toml` left out).
    pub files: Vec<String>,
}

/// `template.toml` as written.
#[derive(Deserialize)]
struct Meta {
    name: String,
    part: u32,
    #[serde(default)]
    order: u32,
    section: String,
    #[serde(default)]
    anchor: String,
    summary: String,
    open: String,
    #[serde(default)]
    try_next: Vec<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    examples: Vec<WikiExample>,
}

/// The names a mod made from a template gets, in place of the template's made-up ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateNames {
    /// `example_mod` and `example_army` become this.
    pub id: String,
    /// "Example Mod" and "Example Army" become this.
    pub name: String,
    /// The folder the mod goes in.
    pub folder: String,
    /// `ExampleMod` (the mod's Lua table) becomes this; `exampleX` becomes its camelCase.
    pub global: String,
    /// "Your Name" becomes this.
    pub author: String,
}

impl TemplateNames {
    /// Names from a mod id and display name: the folder and the Lua global are made from the
    /// name (letters and digits only).
    pub fn new(id: &str, name: &str, author: &str) -> Result<TemplateNames, String> {
        let id: String = id
            .trim()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c.to_ascii_lowercase() } else { '_' })
            .collect();
        if id.is_empty() || !id.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
            return Err("A mod id starts with a letter and has only letters, digits, _ and -.".to_string());
        }
        let name = name.trim().to_string();
        let folder: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')).collect();
        if folder.is_empty() {
            return Err("The mod's name needs at least one letter or digit.".to_string());
        }
        let mut global = String::new();
        let mut upper = true;
        for c in name.chars() {
            if c.is_ascii_alphanumeric() {
                global.push(if upper { c.to_ascii_uppercase() } else { c });
                upper = false;
            } else {
                upper = true;
            }
        }
        if !global.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            global.insert_str(0, "Mod");
        }
        let author = if author.trim().is_empty() { "Your Name" } else { author.trim() };
        Ok(TemplateNames { id, name, folder, global, author: author.to_string() })
    }

    /// The template's own names: rendering with them changes nothing.
    pub fn example() -> TemplateNames {
        TemplateNames {
            id: "example_mod".to_string(),
            name: "Example Mod".to_string(),
            folder: "ExampleMod".to_string(),
            global: "ExampleMod".to_string(),
            author: "Your Name".to_string(),
        }
    }

    fn camel(&self) -> String {
        let mut c = self.global.chars();
        match c.next() {
            Some(f) => f.to_ascii_lowercase().to_string() + c.as_str(),
            None => String::new(),
        }
    }

    /// `text` with the template's made-up names replaced by these.
    pub fn apply(&self, text: &str) -> String {
        if *self == TemplateNames::example() {
            return text.to_string();
        }
        let replaced = text
            .replace("example_mod", &self.id)
            .replace("example_army", &self.id.replace('-', "_"))
            .replace("Example Mod", &self.name)
            .replace("Example Army", &self.name)
            .replace("ExampleMod", &self.global)
            .replace("Your Name", &self.author);
        // `exampleSomething` (a global function) becomes `<camel>Something`.
        let camel = self.camel();
        let mut out = String::with_capacity(replaced.len());
        let mut rest = replaced.as_str();
        while let Some(i) = rest.find("example") {
            let before = rest[..i].chars().last();
            let after = rest[i + "example".len()..].chars().next();
            out.push_str(&rest[..i]);
            let word_start = !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            if word_start && after.is_some_and(|c| c.is_ascii_uppercase()) {
                out.push_str(&camel);
            } else {
                out.push_str("example");
            }
            rest = &rest[i + "example".len()..];
        }
        out.push_str(rest);
        out
    }
}

fn embedded(id: &str) -> Option<&'static [(&'static str, &'static [u8])]> {
    EMBEDDED.iter().find(|(t, _)| *t == id).map(|(_, files)| *files)
}

fn info_of(id: &str, files: &[(&str, &[u8])]) -> Option<TemplateInfo> {
    let meta = files.iter().find(|(p, _)| *p == META_FILE)?;
    let m: Meta = toml::from_str(std::str::from_utf8(meta.1).ok()?).ok()?;
    Some(TemplateInfo {
        id: id.to_string(),
        wiki_url: wiki_url(m.part, &if m.anchor.is_empty() { wiki_anchor(&m.section) } else { m.anchor.clone() }),
        anchor: if m.anchor.is_empty() { wiki_anchor(&m.section) } else { m.anchor },
        name: m.name,
        part: m.part,
        order: m.order,
        section: m.section,
        summary: m.summary,
        open: m.open,
        try_next: m.try_next,
        aliases: m.aliases,
        examples: m.examples,
        files: files.iter().map(|(p, _)| p.to_string()).filter(|p| p != META_FILE).collect(),
    })
}

/// Every template, by wiki part and then its order in the part.
pub fn templates() -> Vec<TemplateInfo> {
    let mut all: Vec<TemplateInfo> = EMBEDDED.iter().filter_map(|(id, files)| info_of(id, files)).collect();
    all.sort_by(|a, b| (a.part, a.order, &a.id).cmp(&(b.part, b.order, &b.id)));
    all
}

/// The template with this id, or the one an older `qm new` name stands for.
pub fn find(id_or_alias: &str) -> Option<TemplateInfo> {
    templates()
        .into_iter()
        .find(|t| t.id == id_or_alias || t.aliases.iter().any(|a| a == id_or_alias))
}

fn is_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// The template's files with `names` in place of its own (paths and text), `template.toml`
/// left out.
pub fn render(id: &str, names: &TemplateNames) -> Result<Vec<(String, Vec<u8>)>, String> {
    let files = embedded(id).ok_or_else(|| format!("There is no template '{id}'."))?;
    Ok(files
        .iter()
        .filter(|(p, _)| *p != META_FILE)
        .map(|(p, bytes)| {
            let path = names.apply(p);
            let body = if is_text(bytes) {
                names.apply(&String::from_utf8_lossy(bytes)).into_bytes()
            } else {
                bytes.to_vec()
            };
            (path, body)
        })
        .collect())
}

/// Makes a new mod folder `<parent>/<names.folder>` from a template, with a README.md when the
/// template has none. Refuses a folder that exists.
pub fn create(parent: &Path, id: &str, names: &TemplateNames) -> Result<PathBuf, String> {
    let mut files = render(id, names)?;
    if !files.iter().any(|(p, _)| p == "README.md") {
        let info = find(id).ok_or_else(|| format!("There is no template '{id}'."))?;
        files.push(("README.md".to_string(), readme(&info, names).into_bytes()));
    }
    let dir = parent.join(&names.folder);
    if dir.exists() {
        return Err(format!("{} already exists.", dir.display()));
    }
    for (rel, bytes) in files {
        crate::library::validate_safe_relative_path(&rel)?;
        let path = dir.join(&rel);
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).map_err(|e| format!("Could not make {}: {e}", p.display()))?;
        }
        fs::write(&path, bytes).map_err(|e| format!("Could not write {rel}: {e}"))?;
    }
    Ok(dir)
}

/// The README a new mod starts with: what it is, and where its template's ability is explained.
fn readme(info: &TemplateInfo, names: &TemplateNames) -> String {
    format!(
        "# {name}

{name} for Mud and Blood. Needs [QuarterMaster](https://gamiseus.github.io/).

Started from the template \"{template}\": {summary}
The modding wiki explains it: {wiki}

## Working on it

- Check it against the game: `qm lint .`, or the Mod Engine's Check mod.
- Try it: put this folder in your workspace mods folder and use the Mod Engine's Test in game.
- Release it: the Mod Engine's Release view, or raise `version` in `mod.json` and run `qm pack .`.
",
        name = names.name,
        template = info.name,
        summary = names.apply(&info.summary),
        wiki = info.wiki_url,
    )
}

/// The anchor a wiki heading gets on the site (GitHub's rule: lower case, spaces to `-`,
/// punctuation dropped).
/// The address of a section of the published wiki, by part and heading id.
pub fn wiki_url(part: u32, anchor: &str) -> String {
    match WIKI_PAGES.get((part as usize).wrapping_sub(1)) {
        Some(page) => format!("{WIKI_URL}{page}/#{anchor}"),
        None => format!("{WIKI_URL}all/#{anchor}"),
    }
}

pub fn wiki_anchor(heading: &str) -> String {
    heading
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn names_replace_the_wiki_names() {
        let n = TemplateNames::new("field_radio", "Field Radio", "Ada").unwrap();
        assert_eq!(n.folder, "FieldRadio");
        assert_eq!(n.global, "FieldRadio");
        assert_eq!(
            n.apply("id = \"example_mod:ping\" -- ExampleMod.onPing, exampleStartOperation, an example, Example Mod by Your Name"),
            "id = \"field_radio:ping\" -- FieldRadio.onPing, fieldRadioStartOperation, an example, Field Radio by Ada"
        );
        assert_eq!(n.apply("example_army / Example Army"), "field_radio / Field Radio");
        assert_eq!(TemplateNames::example().apply("exampleX example_mod ExampleMod"), "exampleX example_mod ExampleMod");
        assert!(TemplateNames::new("9lives", "Nine", "").is_err());
        assert_eq!(TemplateNames::new("x", "9 Lives", "").unwrap().global, "Mod9Lives");
    }

    #[test]
    fn anchors_follow_the_site() {
        assert_eq!(wiki_anchor("Names: `[names]`"), "names-names");
        assert_eq!(wiki_anchor("Mod Operations: cards and pages"), "mod-operations-cards-and-pages");
    }
}
