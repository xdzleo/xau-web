//! Who may talk to the server.
//!
//! The server holds the signed-in accounts, so a page on the internet must
//! never be able to drive it just because the user's browser can reach
//! `localhost`. Three independent checks, all of which must pass:
//!
//! 1. **Host**: the request must name this machine (`localhost`,
//!    `127.0.0.1`, `[::1]`), which defeats DNS rebinding. With `--lan` any
//!    name is accepted, and the key is the only gate.
//! 2. **Key**: a random secret, kept in the data folder, carried by an
//!    `HttpOnly; SameSite=Strict` cookie that only the printed link sets.
//! 3. **Origin**: a browser-made POST must come from this very origin, and
//!    must carry `x-xau: 1`, a header a cross-site page cannot add without
//!    a CORS preflight, which the server never grants.

use std::path::Path;

use axum::http::{header, HeaderMap};

/// A fresh 256-bit key as lowercase hex.
pub fn new_key() -> String {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).expect("the OS random source");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The key of this install, created on first run.
pub fn load_or_create_key(dir: &Path) -> std::io::Result<String> {
    let path = dir.join("web-key.txt");
    if let Ok(k) = std::fs::read_to_string(&path) {
        let k = k.trim().to_string();
        if k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(k);
        }
    }
    std::fs::create_dir_all(dir)?;
    let k = new_key();
    std::fs::write(&path, &k)?;
    Ok(k)
}

/// Equal strings, in time that does not depend on where they differ.
pub fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn host_of(h: &HeaderMap) -> Option<&str> {
    h.get(header::HOST).and_then(|v| v.to_str().ok())
}

pub fn host_ok(h: &HeaderMap, lan: bool) -> bool {
    let Some(host) = host_of(h) else { return false };
    if lan {
        return true;
    }
    // `[::1]:8787` has colons of its own; the port is after the last one.
    let name = host.rsplit_once(':').map_or(host, |(n, _)| n);
    matches!(name, "localhost" | "127.0.0.1" | "[::1]")
}

pub fn cookie_name(port: u16) -> String {
    format!("xau_key_{port}")
}

pub fn cookie_ok(h: &HeaderMap, port: u16, key: &str) -> bool {
    let want = cookie_name(port);
    h.get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .any(|(k, v)| k == want && same(v, key))
}

/// A POST made by a page must come from this origin and carry `x-xau`.
pub fn post_ok(h: &HeaderMap) -> bool {
    let marked = h.get("x-xau").and_then(|v| v.to_str().ok()) == Some("1");
    let origin_ok = match h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        None => true,
        Some(o) => host_of(h).is_some_and(|host| o == format!("http://{host}")),
    };
    marked && origin_ok
}

/// A WebSocket a page opens: browsers always send its Origin, and it must be this one
/// (a cross-site page can open a WebSocket with the user's cookies otherwise).
pub fn ws_origin_ok(h: &HeaderMap) -> bool {
    match (h.get(header::ORIGIN).and_then(|v| v.to_str().ok()), host_of(h)) {
        (Some(o), Some(host)) => o == format!("http://{host}"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn only_this_machine_may_be_named() {
        assert!(host_ok(&headers(&[("host", "localhost:8787")]), false));
        assert!(host_ok(&headers(&[("host", "127.0.0.1:8787")]), false));
        assert!(host_ok(&headers(&[("host", "[::1]:8787")]), false));
        assert!(!host_ok(&headers(&[("host", "evil.example:8787")]), false), "DNS rebinding");
        assert!(!host_ok(&headers(&[("host", "localhost.evil.example")]), false));
        assert!(!host_ok(&HeaderMap::new(), false));
        assert!(host_ok(&headers(&[("host", "192.168.0.5:8787")]), true), "--lan");
    }

    #[test]
    fn the_cookie_must_carry_this_ports_key() {
        let key = new_key();
        let h = headers(&[("cookie", &format!("a=b; xau_key_8787={key}; c=d"))]);
        assert!(cookie_ok(&h, 8787, &key));
        assert!(!cookie_ok(&h, 8788, &key), "another instance's cookie is not ours");
        assert!(!cookie_ok(&h, 8787, &new_key()));
        assert!(!cookie_ok(&HeaderMap::new(), 8787, &key));
    }

    #[test]
    fn a_post_needs_the_marker_and_the_same_origin() {
        let ok = headers(&[("host", "localhost:8787"), ("origin", "http://localhost:8787"), ("x-xau", "1")]);
        assert!(post_ok(&ok));
        let cross = headers(&[("host", "localhost:8787"), ("origin", "https://evil.example"), ("x-xau", "1")]);
        assert!(!post_ok(&cross));
        let unmarked = headers(&[("host", "localhost:8787"), ("origin", "http://localhost:8787")]);
        assert!(!post_ok(&unmarked));
    }

    #[test]
    fn a_websocket_needs_this_origin() {
        assert!(ws_origin_ok(&headers(&[("host", "localhost:8787"), ("origin", "http://localhost:8787")])));
        assert!(!ws_origin_ok(&headers(&[("host", "localhost:8787"), ("origin", "https://evil.example")])));
        assert!(!ws_origin_ok(&headers(&[("host", "localhost:8787")])));
    }

    #[test]
    fn keys_are_random_hex_and_compared_whole() {
        let (a, b) = (new_key(), new_key());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(same(&a, &a) && !same(&a, &b) && !same(&a, &a[..63]));
    }
}
