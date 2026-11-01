//! Navigation guard: webviews may only show the app's own pages. External links go through
//! `tauri-plugin-opener` (https only), never through webview navigation.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime, Url};

/// True for the bundled app origin (`tauri://localhost`, `http(s)://tauri.localhost`),
/// `about:blank`, and the dev server origin when one is given.
pub fn is_app_url(url: &Url, dev_url: Option<&Url>) -> bool {
    match url.scheme() {
        "tauri" => url.host_str() == Some("localhost") && url.port().is_none(),
        "http" | "https" if url.host_str() == Some("tauri.localhost") => url.port().is_none(),
        "about" => url.path() == "blank",
        _ => dev_url.is_some_and(|dev| dev.origin() == url.origin()),
    }
}

/// Plugin that cancels every navigation away from the app origin.
pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("qm-navigation")
        .on_navigation(|webview, url| {
            // The dev server only counts in dev builds; release serves the bundled assets.
            let dev_url = if tauri::is_dev() {
                webview.config().build.dev_url.clone()
            } else {
                None
            };
            let allowed = is_app_url(url, dev_url.as_ref());
            if !allowed {
                tracing::warn!(
                    "blocked navigation to {}",
                    url.origin().ascii_serialization()
                );
            }
            allowed
        })
        .build()
}
