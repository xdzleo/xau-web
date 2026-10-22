//! Bakes every file under `ui/` into the binary, so the web version is one
//! executable with nothing to install beside it.

use std::{env, fs, path::Path};

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "ttf" => "font/ttf",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn main() {
    let ui = Path::new("ui");
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=build.rs");

    let mut files = Vec::new();
    walk(ui, ui, &mut files);
    files.sort();

    let mut code = String::from("pub static ASSETS: &[Asset] = &[\n");
    for rel in &files {
        code.push_str(&format!(
            "    Asset {{ path: {rel:?}, mime: {:?}, bytes: include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/ui/\", {rel:?})) }},\n",
            mime(rel)
        ));
    }
    code.push_str("];\n");

    let out = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("assets_gen.rs");
    fs::write(out, code).expect("write assets_gen.rs");
}
