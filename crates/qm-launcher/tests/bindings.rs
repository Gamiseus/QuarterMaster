//! Keeps `launcher-ui/src/lib/bindings.ts` in sync with the Rust command layer.
use std::path::PathBuf;

fn bindings_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../launcher-ui/src/lib/bindings.ts")
}

#[test]
fn bindings_are_up_to_date() {
    let generated = qm_launcher_lib::bindings::export_bindings().expect("export bindings");
    let path = bindings_path();

    if std::env::var("UPDATE_BINDINGS").as_deref() == Ok("1") {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("create bindings dir");
        }
        std::fs::write(&path, &generated).expect("write bindings.ts");
        return;
    }

    let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    // Line endings may be rewritten on checkout.
    let normalize = |s: &str| s.replace("\r\n", "\n");
    assert!(
        normalize(&on_disk) == normalize(&generated),
        "bindings.ts is stale; run: $env:UPDATE_BINDINGS=1; cargo test -p qm-launcher --test bindings"
    );
}
