fn main() {
    tauri_build::build();
    // tauri-build links the Windows resource (Common Controls v6 manifest) into bins only;
    // integration tests that load tauri need it too or they fail with STATUS_ENTRYPOINT_NOT_FOUND.
    // MSVC only (resource.lib is the MSVC output name). `rustc-link-arg-tests` covers
    // [[test]] targets (tests/*.rs) only, not the lib's unit tests.
    // Cargo rejects the instruction when the package has no test target, and `tests/` is kept
    // local (not in the repository), so only emit it when the suite is there.
    println!("cargo:rerun-if-changed=tests/suite/main.rs");
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default();
    let has_suite = std::path::Path::new(&manifest_dir)
        .join("tests/suite/main.rs")
        .exists();
    if let (true, Some(out_dir)) = (has_suite, std::env::var_os("OUT_DIR")) {
        let res = std::path::Path::new(&out_dir).join("resource.lib");
        println!("cargo:rustc-link-arg-tests={}", res.display());
    }
}
