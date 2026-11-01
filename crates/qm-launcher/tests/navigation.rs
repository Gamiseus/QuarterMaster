//! The navigation guard keeps the webview on the app origin.
use qm_launcher_lib::navigation::is_app_url;
use tauri::Url;

fn url(s: &str) -> Url {
    Url::parse(s).unwrap_or_else(|e| panic!("bad test url {s}: {e}"))
}

#[test]
fn allows_the_bundled_app_origin() {
    for ok in [
        "tauri://localhost",
        "tauri://localhost/index.html",
        "http://tauri.localhost/",
        "http://tauri.localhost/assets/app.js",
        "https://tauri.localhost/",
        "about:blank",
    ] {
        assert!(is_app_url(&url(ok), None), "{ok} should be allowed");
    }
}

#[test]
fn blocks_everything_else() {
    for bad in [
        "https://example.com/",
        "http://localhost:5173/",
        "https://tauri.localhost.evil.com/",
        "http://tauri.localhost:8080/",
        "tauri://evil/",
        "file:///C:/Windows/System32/cmd.exe",
        "javascript:alert(1)",
        "data:text/html,<p>x</p>",
        "about:srcdoc",
    ] {
        assert!(!is_app_url(&url(bad), None), "{bad} should be blocked");
    }
}

#[test]
fn allows_only_the_dev_server_origin_when_given() {
    let dev = url("http://localhost:5173");
    assert!(is_app_url(&url("http://localhost:5173/"), Some(&dev)));
    assert!(is_app_url(
        &url("http://localhost:5173/src/main.ts"),
        Some(&dev)
    ));
    assert!(!is_app_url(&url("http://localhost:5174/"), Some(&dev)));
    assert!(!is_app_url(&url("https://localhost:5173/"), Some(&dev)));
    assert!(!is_app_url(&url("https://example.com/"), Some(&dev)));
}
