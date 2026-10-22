fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let def_path = format!("{}/version.def", manifest_dir.replace('\\', "/"));
    println!("cargo:rustc-cdylib-link-arg=/DEF:{}", def_path);
}
