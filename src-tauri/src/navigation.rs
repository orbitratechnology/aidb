//! Keeps the webview on the app's own content.
//!
//! The UI is a local single-page app and never needs to navigate the webview elsewhere. Any other
//! navigation (an injected link, a redirect, a scripted `location` change) is blocked. External
//! pages open in the system browser through the `open_external_url` command instead.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime, Url};

/// Plugin that rejects top-level navigation away from the bundled app.
pub fn guard<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("navigation-guard")
        .on_navigation(|webview, url| {
            let dev_url = if tauri::is_dev() {
                webview.config().build.dev_url.as_ref()
            } else {
                None
            };
            let allowed = is_app_url(url, dev_url);
            if !allowed {
                // Log only scheme and host: paths and queries can carry secrets.
                tracing::warn!(
                    scheme = url.scheme(),
                    host = url.host_str().unwrap_or_default(),
                    "blocked navigation away from the app"
                );
            }
            allowed
        })
        .build()
}

/// Whether `url` belongs to the bundled app or, during development, the dev server.
fn is_app_url(url: &Url, dev_url: Option<&Url>) -> bool {
    match url.scheme() {
        // Bundled assets: `tauri://localhost` on macOS and Linux.
        "tauri" => url.host_str() == Some("localhost"),
        // Bundled assets on Windows and Android.
        "http" | "https" if url.host_str() == Some("tauri.localhost") => true,
        "about" => url.path() == "blank",
        _ => dev_url.is_some_and(|dev| dev.origin() == url.origin()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        text.parse().expect("test URL parses")
    }

    #[test]
    fn allows_bundled_app_urls_on_every_platform() {
        assert!(is_app_url(&url("tauri://localhost/"), None));
        assert!(is_app_url(
            &url("tauri://localhost/index.html#/settings"),
            None
        ));
        assert!(is_app_url(&url("http://tauri.localhost/"), None));
        assert!(is_app_url(
            &url("https://tauri.localhost/assets/x.js"),
            None
        ));
        assert!(is_app_url(&url("about:blank"), None));
    }

    #[test]
    fn blocks_everything_else() {
        for blocked in [
            "https://example.com/",
            "http://localhost:5173/",
            "tauri://evil/",
            "http://tauri.localhost.evil.com/",
            "file:///etc/passwd",
            "data:text/html,hi",
            "about:config",
        ] {
            assert!(
                !is_app_url(&url(blocked), None),
                "{blocked} must be blocked"
            );
        }
    }

    #[test]
    fn allows_only_the_exact_dev_server_origin_in_development() {
        let dev = url("http://localhost:5173");

        assert!(is_app_url(&url("http://localhost:5173/#/ask"), Some(&dev)));
        assert!(!is_app_url(&url("http://localhost:5174/"), Some(&dev)));
        assert!(!is_app_url(&url("https://localhost:5173/"), Some(&dev)));
    }
}
