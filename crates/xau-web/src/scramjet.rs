//! The files of the in-browser sign-in, served to the person's browser.
//!
//! [Scramjet](https://github.com/MercuryWorkshop/scramjet) (AGPL-3.0) rewrites a web page so it
//! can run inside another one, and the libcurl transport does the TLS in WebAssembly. They are
//! not part of this repository: the container image installs pinned versions with npm (see
//! `docker/scramjet`) into a folder, and `XAU_SCRAMJET_DIR` points at it. With no folder the
//! in-browser sign-in is simply off and the sign-in the server runs for the person (a picture of
//! a browser on this machine) is used for everyone.
//!
//! The folder holds exactly these files, and nothing else is ever served from it. They are kept
//! in memory and served under a URL that carries a hash of their content, so a browser keeps them
//! forever and a new version is a new URL.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;

/// Where each file comes from in the npm packages, and where the page finds it.
const FILES: &[(&str, &str)] = &[
    ("controller/controller.api.js", "text/javascript; charset=utf-8"),
    ("controller/controller.inject.js", "text/javascript; charset=utf-8"),
    ("controller/controller.sw.js", "text/javascript; charset=utf-8"),
    ("scram/scramjet.js", "text/javascript; charset=utf-8"),
    ("scram/scramjet.wasm", "application/wasm"),
    ("clients/libcurl-client.js", "text/javascript; charset=utf-8"),
];

pub struct File {
    pub mime: &'static str,
    pub bytes: Bytes,
}

pub struct Files {
    /// A short hash of every file: the URL they are served under.
    pub version: String,
    map: HashMap<&'static str, File>,
}

/// FNV-1a: not a security hash, only a name that changes when the content does.
fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl Files {
    /// The folder from `XAU_SCRAMJET_DIR`, or `scramjet` next to the executable; `None` (with a
    /// line in the log) when it is missing or incomplete.
    pub fn load() -> Option<Arc<Self>> {
        let dir = std::env::var_os("XAU_SCRAMJET_DIR").map(PathBuf::from).or_else(|| {
            let beside = std::env::current_exe().ok()?.parent()?.join("scramjet");
            beside.is_dir().then_some(beside)
        })?;
        Self::from_dir(&dir)
    }

    fn from_dir(dir: &std::path::Path) -> Option<Arc<Self>> {
        let mut map = HashMap::new();
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for (path, mime) in FILES {
            match std::fs::read(dir.join(path)) {
                Ok(b) => {
                    hash = fnv(fnv(hash, path.as_bytes()), &b);
                    map.insert(*path, File { mime, bytes: Bytes::from(b) });
                }
                Err(e) => {
                    tracing::warn!("login no navegador desligado: falta {path} em {} ({e})", dir.display());
                    return None;
                }
            }
        }
        Some(Arc::new(Self { version: format!("{hash:016x}")[..12].to_string(), map }))
    }

    pub fn get(&self, path: &str) -> Option<&File> {
        self.map.get(path)
    }

    /// The service worker that carries the page's requests: it only answers the ones for the
    /// sign-in page's own prefix (`/~/sj/`), and leaves everything else of the site alone.
    ///
    /// The browser stops an idle worker after some seconds and starts it again for the next
    /// request, and a started worker knows no controller until the page answers its call (about
    /// 100 ms later). The request that woke it (the email's lookup, a link) must wait for that:
    /// let through, it reaches this server instead of Microsoft and the page hangs.
    pub fn service_worker(&self) -> String {
        format!(
            "importScripts(\"/sj/{v}/controller/controller.sw.js\");\n\
             const revived = async (e) => {{\n\
             \tfor (let i = 0; i < 250 && !$scramjetController.shouldRoute(e); i++) {{\n\
             \t\tawait new Promise((r) => setTimeout(r, 20));\n\
             \t}}\n\
             \tif ($scramjetController.shouldRoute(e)) return $scramjetController.route(e);\n\
             \treturn new Response(\"service worker sem o controlador\", {{ status: 503 }});\n\
             }};\n\
             addEventListener(\"fetch\", (e) => {{\n\
             \tif ($scramjetController.shouldRoute(e)) {{\n\
             \t\te.respondWith($scramjetController.route(e));\n\
             \t}} else if (new URL(e.request.url).pathname.startsWith(\"/~/sj/\")) {{\n\
             \t\te.respondWith(revived(e));\n\
             \t}}\n\
             }});\n",
            v = self.version
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(with: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xau-sj-test-{}-{}", std::process::id(), with.len()));
        let _ = std::fs::remove_dir_all(&dir);
        for p in with {
            let f = dir.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, format!("// {p}")).unwrap();
        }
        dir
    }

    #[test]
    fn an_incomplete_folder_turns_the_feature_off() {
        let dir = folder(&["scram/scramjet.js"]);
        assert!(Files::from_dir(&dir).is_none());
    }

    #[test]
    fn only_the_listed_files_are_served_and_the_version_follows_the_content() {
        let all: Vec<&str> = FILES.iter().map(|(p, _)| *p).collect();
        let dir = folder(&all);
        std::fs::write(dir.join("secret.txt"), "no").unwrap();
        let a = Files::from_dir(&dir).expect("complete folder");
        assert!(a.get("controller/controller.sw.js").is_some());
        assert!(a.get("secret.txt").is_none());
        assert!(a.get("../secret.txt").is_none());
        assert_eq!(a.get("scram/scramjet.wasm").unwrap().mime, "application/wasm");
        std::fs::write(dir.join("scram/scramjet.js"), "// changed").unwrap();
        let b = Files::from_dir(&dir).unwrap();
        assert_ne!(a.version, b.version);
        assert!(b.service_worker().contains(&format!("/sj/{}/controller/controller.sw.js", b.version)));
    }
}
