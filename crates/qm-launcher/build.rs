fn main() {
    tauri_build::build();
    // tauri-build links the Windows resource (Common Controls v6 manifest) into bins only;
    // integration tests that load tauri need it too or they fail with STATUS_ENTRYPOINT_NOT_FOUND.
    // MSVC only (resource.lib is the MSVC output name). `rustc-link-arg-tests` covers
    // [[test]] targets (tests/*.rs) only, not the lib's unit tests.
    if let Some(out_dir) = std::env::var_os("OUT_DIR") {
        let res = std::path::Path::new(&out_dir).join("resource.lib");
        println!("cargo:rustc-link-arg-tests={}", res.display());
    }
}
