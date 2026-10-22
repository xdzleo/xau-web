//! Who may talk to the server.
//!
//! The server holds the signed-in accounts, so a page on the internet must
//! never be able to drive it just because the user's browser can reach
//! `localhost`. Three independent checks, all of which must pass:
//!
//! 1. **Host**: the request must name this machine (`localhost`,
//!    `127.0.0.1`, `[::1]`), which defeats DNS rebinding. On a server open to
//!    the internet (`XAU_PUBLIC_HOSTS`), the names of its own domain are
//!    accepted too, and only those. With `--lan` any name is accepted, and the
//!    key is the only gate.
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

/// Where the server is reachable, and so what a request may call it.
#[derive(Clone, Default)]
pub struct Access {
    /// Any name is accepted (`--lan`): the key is the only gate.
    pub lan: bool,
    /// The domain names of a server open to the internet; empty on a computer.
    pub hosts: std::sync::Arc<Vec<String>>,
}

impl Access {
    /// A server behind a TLS proxy (the cloud): its own domain, reached over https.
    pub fn public(&self) -> bool {
        !self.hosts.is_empty()
    }

    pub fn host_ok(&self, h: &HeaderMap) -> bool {
        let Some(host) = host_of(h) else { return false };
        if self.lan {
            return true;
        }
        // `[::1]:8787` has colons of its own; the port is after the last one.
        let name = host.rsplit_once(':').map_or(host, |(n, _)| n);
        matches!(name, "localhost" | "127.0.0.1" | "[::1]") || self.hosts.iter().any(|d| d.eq_ignore_ascii_case(name))
    }

    /// ``https`` when the proxy in front says it terminated TLS, else ``http``. Trusted only on a
    /// public server: there a request cannot reach us except through that proxy.
    pub fn scheme(&self, h: &HeaderMap) -> &'static str {
        let https = h.get("x-forwarded-proto").and_then(|v| v.to_str().ok()).is_some_and(|v| v.eq_ignore_ascii_case("https"));
        if self.public() && https {
            "https"
        } else {
            "http"
        }
    }

    pub fn cookie_name(&self, port: u16) -> String {
        if self.public() {
            "xau_key".to_string()
        } else {
            format!("xau_key_{port}")
        }
    }

    pub fn cookie_ok(&self, h: &HeaderMap, port: u16, key: &str) -> bool {
        let want = self.cookie_name(port);
        h.get_all(header::COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(';'))
            .filter_map(|kv| kv.trim().split_once('='))
            .any(|(k, v)| k == want && same(v, key))
    }

    fn origin_is_ours(&self, h: &HeaderMap, o: &str) -> bool {
        host_of(h).is_some_and(|host| o == format!("{}://{host}", self.scheme(h)))
    }

    /// A POST made by a page must come from this origin and carry `x-xau`.
    pub fn post_ok(&self, h: &HeaderMap) -> bool {
        let marked = h.get("x-xau").and_then(|v| v.to_str().ok()) == Some("1");
        let origin_ok = match h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            None => true,
            Some(o) => self.origin_is_ours(h, o),
        };
        marked && origin_ok
    }

    /// A WebSocket a page opens: browsers always send its Origin, and it must be this one
    /// (a cross-site page can open a WebSocket with the user's cookies otherwise).
    pub fn ws_origin_ok(&self, h: &HeaderMap) -> bool {
        h.get(header::ORIGIN).and_then(|v| v.to_str().ok()).is_some_and(|o| self.origin_is_ours(h, o))
    }
}

/// The key of this install, from the environment when it is set there (a cloud server
/// keeps its secrets in variables), else the one kept in the data folder.
pub fn key_from_env() -> Option<String> {
    let k = std::env::var("XAU_KEY").ok()?.trim().to_string();
    (k.len() >= 32 && k.bytes().all(|b| b.is_ascii_graphic())).then_some(k)
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

    fn local() -> Access {
        Access::default()
    }

    fn lan() -> Access {
        Access { lan: true, ..Access::default() }
    }

    fn public() -> Access {
        Access { lan: false, hosts: std::sync::Arc::new(vec!["rewards.example.com".to_string()]) }
    }

    #[test]
    fn only_this_machine_may_be_named() {
        assert!(local().host_ok(&headers(&[("host", "localhost:8787")])));
        assert!(local().host_ok(&headers(&[("host", "127.0.0.1:8787")])));
        assert!(local().host_ok(&headers(&[("host", "[::1]:8787")])));
        assert!(!local().host_ok(&headers(&[("host", "evil.example:8787")])), "DNS rebinding");
        assert!(!local().host_ok(&headers(&[("host", "localhost.evil.example")])));
        assert!(!local().host_ok(&HeaderMap::new()));
        assert!(lan().host_ok(&headers(&[("host", "192.168.0.5:8787")])), "--lan");
    }

    #[test]
    fn a_public_server_answers_to_its_own_domain_only() {
        assert!(public().host_ok(&headers(&[("host", "rewards.example.com")])));
        assert!(public().host_ok(&headers(&[("host", "Rewards.Example.com")])));
        assert!(!public().host_ok(&headers(&[("host", "evil.example.com")])));
        assert!(!public().host_ok(&headers(&[("host", "rewards.example.com.evil.net")])));
        assert!(!public().host_ok(&headers(&[("host", "xrewards.example.com")])));
    }

    #[test]
    fn the_cookie_must_carry_this_ports_key() {
        let key = new_key();
        let h = headers(&[("cookie", &format!("a=b; xau_key_8787={key}; c=d"))]);
        assert!(local().cookie_ok(&h, 8787, &key));
        assert!(!local().cookie_ok(&h, 8788, &key), "another instance's cookie is not ours");
        assert!(!local().cookie_ok(&h, 8787, &new_key()));
        assert!(!local().cookie_ok(&HeaderMap::new(), 8787, &key));
        // On a public server the port is the proxy's business: one name, whatever the port.
        let p = headers(&[("cookie", &format!("xau_key={key}"))]);
        assert!(public().cookie_ok(&p, 8080, &key) && public().cookie_ok(&p, 3000, &key));
    }

    #[test]
    fn a_post_needs_the_marker_and_the_same_origin() {
        let ok = headers(&[("host", "localhost:8787"), ("origin", "http://localhost:8787"), ("x-xau", "1")]);
        assert!(local().post_ok(&ok));
        let cross = headers(&[("host", "localhost:8787"), ("origin", "https://evil.example"), ("x-xau", "1")]);
        assert!(!local().post_ok(&cross));
        let unmarked = headers(&[("host", "localhost:8787"), ("origin", "http://localhost:8787")]);
        assert!(!local().post_ok(&unmarked));
    }

    #[test]
    fn behind_the_proxy_the_origin_is_https() {
        let h = |origin: &str, proto: &str| {
            headers(&[("host", "rewards.example.com"), ("origin", origin), ("x-xau", "1"), ("x-forwarded-proto", proto)])
        };
        assert!(public().post_ok(&h("https://rewards.example.com", "https")));
        assert!(!public().post_ok(&h("http://rewards.example.com", "https")), "the page is https: an http origin is not it");
        assert!(!public().post_ok(&h("https://evil.example", "https")));
        // A computer's server never believes the header: there is no proxy in front of it.
        assert!(!local().post_ok(&headers(&[("host", "localhost:8787"), ("origin", "https://localhost:8787"), ("x-xau", "1"), ("x-forwarded-proto", "https")])));
    }

    #[test]
    fn a_websocket_needs_this_origin() {
        assert!(local().ws_origin_ok(&headers(&[("host", "localhost:8787"), ("origin", "http://localhost:8787")])));
        assert!(!local().ws_origin_ok(&headers(&[("host", "localhost:8787"), ("origin", "https://evil.example")])));
        assert!(!local().ws_origin_ok(&headers(&[("host", "localhost:8787")])));
        let p = headers(&[("host", "rewards.example.com"), ("origin", "https://rewards.example.com"), ("x-forwarded-proto", "https")]);
        assert!(public().ws_origin_ok(&p));
    }
    #[test]
    fn keys_are_random_hex_and_compared_whole() {
        let (a, b) = (new_key(), new_key());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(same(&a, &a) && !same(&a, &b) && !same(&a, &a[..63]));
    }
}
