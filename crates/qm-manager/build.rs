//! Embeds the mod templates (`Quartermaster/templates/<id>/`) into the crate, so `qm new` and
//! the launcher's Mod Engine always carry the templates of the loader they ship with.
use std::fs;
use std::path::{Path, PathBuf};

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            files_under(&p, out);
        } else if p.is_file() {
            println!("cargo:rerun-if-changed={}", p.display());
            out.push(p);
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let root = manifest.join("..").join("..").join("templates");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut code = String::from(
        "/// A template's files, as (path with `/`, bytes).\n\
         pub type TemplateFiles = &'static [(&'static str, &'static [u8])];\n\
         /// Every template: its id and its files.\n\
         pub static EMBEDDED: &[(&str, TemplateFiles)] = &[\n",
    );
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect())
        .unwrap_or_default();
    dirs.sort();
    for dir in dirs {
        let id = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut files = Vec::new();
        files_under(&dir, &mut files);
        code.push_str(&format!("    ({id:?}, &[\n"));
        for f in files {
            let rel = f
                .strip_prefix(&dir)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            code.push_str(&format!(
                "        ({rel:?}, include_bytes!({:?})),\n",
                f.to_string_lossy()
            ));
        }
        code.push_str("    ]),\n");
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("templates_gen.rs");
    fs::write(out, code).unwrap_or_else(|e| panic!("could not write templates_gen.rs: {e}"));
}
