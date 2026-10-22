//! The web UI, baked into the binary by `build.rs`.
//!
//! For working on the UI, `XAU_WEB_UI_DIR=<path to ui/>` serves the files
//! from disk instead, so a change shows on reload without rebuilding.

use std::borrow::Cow;

pub struct Asset {
    pub path: &'static str,
    pub mime: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/assets_gen.rs"));

/// A served file: its type and bytes.
pub struct File {
    pub mime: &'static str,
    pub bytes: Cow<'static, [u8]>,
}

fn mime_of(path: &str) -> &'static str {
    ASSETS
        .iter()
        .find(|a| a.path == path)
        .map(|a| a.mime)
        .unwrap_or_else(|| match path.rsplit('.').next().unwrap_or("") {
            "html" => "text/html; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "js" | "mjs" => "text/javascript; charset=utf-8",
            "svg" => "image/svg+xml",
            "ttf" => "font/ttf",
            _ => "application/octet-stream",
        })
}

/// A file of the UI by its path under `ui/`, e.g. `js/app.js`.
pub fn get(path: &str) -> Option<File> {
    if let Some(dir) = std::env::var_os("XAU_WEB_UI_DIR") {
        // Dev only. Never a path that leaves the folder.
        if path.split('/').any(|p| p == ".." || p.contains('\\') || p.contains(':')) {
            return None;
        }
        let bytes = std::fs::read(std::path::Path::new(&dir).join(path)).ok()?;
        return Some(File { mime: mime_of(path), bytes: Cow::Owned(bytes) });
    }
    ASSETS
        .iter()
        .find(|a| a.path == path)
        .map(|a| File { mime: a.mime, bytes: Cow::Borrowed(a.bytes) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_baked_in_ui_has_its_entry_points() {
        for p in ["index.html", "style.css", "js/app.js", "js/icons.js", "favicon.svg"] {
            assert!(ASSETS.iter().any(|a| a.path == p), "{p} is missing from ui/");
        }
    }

    #[test]
    fn a_path_cannot_leave_the_ui_folder() {
        std::env::set_var("XAU_WEB_UI_DIR", ".");
        assert!(get("../Cargo.toml").is_none());
        assert!(get("js/../../Cargo.toml").is_none());
        assert!(get("C:/Windows/win.ini").is_none());
        std::env::remove_var("XAU_WEB_UI_DIR");
    }
}
