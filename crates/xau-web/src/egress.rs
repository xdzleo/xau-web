//! Where the server's own traffic leaves from.
//!
//! On a cloud server everything meant for Microsoft (the sign-in page, Xbox Live,
//! the Store, the farm) must leave through Cloudflare's network (WARP in proxy mode,
//! a SOCKS proxy on this machine), never from the host's own datacenter address. The
//! proxy is set for every HTTP client of the core through the environment, and for the
//! browser that runs the sign-in page by its command line (see `cdphost`).
//!
//! The rule is fail-closed: until the exit has been *checked* to be Cloudflare's, the
//! calls that reach Microsoft are refused rather than sent the direct way. The check is
//! Cloudflare's own trace page, fetched through the proxy: `warp=on|plus` (or `gateway=on`)
//! means the request came out of WARP, and it must not show the same address as the same
//! request made without the proxy (that would mean the proxy was bypassed).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const TRACE: &str = "https://www.cloudflare.com/cdn-cgi/trace";

#[derive(Clone, Default, Debug)]
pub struct State {
    pub ok: bool,
    pub ip: String,
    pub colo: String,
    pub warp: String,
    pub detail: String,
    /// When the proxy last answered with a confirmed Cloudflare exit.
    pub last_ok: Option<Instant>,
}

pub struct Egress {
    proxy: Option<String>,
    required: bool,
    state: Mutex<State>,
}

/// `key=value` lines of Cloudflare's trace page.
fn parse_trace(body: &str) -> HashMap<String, String> {
    body.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect()
}

/// What the trace says about the exit: `Ok(())` when it is Cloudflare's WARP/Gateway.
fn judge(proxied: &HashMap<String, String>, direct_ip: Option<&str>) -> Result<(), String> {
    let warp = proxied.get("warp").map_or("off", String::as_str);
    let gateway = proxied.get("gateway").map_or("off", String::as_str);
    if !(matches!(warp, "on" | "plus") || gateway == "on") {
        return Err(format!("a saída não é a do WARP (warp={warp}, gateway={gateway})"));
    }
    let ip = proxied.get("ip").map_or("", String::as_str);
    if direct_ip.is_some_and(|d| !d.is_empty() && d == ip) {
        return Err("a saída pelo proxy tem o mesmo endereço da saída direta: o proxy está sendo ignorado".into());
    }
    Ok(())
}

impl Egress {
    /// From the environment. A server (``server_mode``) must have a proxy unless it was told
    /// outright that the direct way is fine (``XAU_ALLOW_DIRECT_EGRESS``).
    pub fn from_env(server_mode: bool) -> Arc<Self> {
        let proxy = std::env::var("XAU_EGRESS_PROXY").ok().map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
        let allow_direct = std::env::var_os("XAU_ALLOW_DIRECT_EGRESS").is_some();
        let required = proxy.is_some() || (server_mode && !allow_direct);
        Arc::new(Self { proxy, required, state: Mutex::new(State::default()) })
    }

    /// Point every HTTP client of this process at the proxy (reqwest reads these), and keep
    /// the loopback (the browser's DevTools, this server) off it.
    pub fn apply_to_environment(&self) {
        if let Some(p) = &self.proxy {
            for k in ["ALL_PROXY", "all_proxy", "HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
                std::env::set_var(k, p);
            }
            for k in ["NO_PROXY", "no_proxy"] {
                std::env::set_var(k, "127.0.0.1,localhost,::1");
            }
        }
    }

    /// Whether a call to Microsoft may go out now.
    pub fn require(&self) -> Result<(), String> {
        if !self.required {
            return Ok(());
        }
        if self.proxy.is_none() {
            return Err("este servidor só fala com a Microsoft pela Cloudflare (WARP), e ela não está configurada".into());
        }
        let s = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if s.ok {
            Ok(())
        } else {
            Err(format!("a saída pela Cloudflare (WARP) ainda não está confirmada: {}", if s.detail.is_empty() { "verificando" } else { &s.detail }))
        }
    }

    pub fn snapshot(&self) -> Value {
        let s = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        json!({ "required": self.required, "proxy": self.proxy.is_some(), "ok": s.ok || !self.required, "colo": s.colo, "warp": s.warp, "ip": s.ip, "detail": s.detail })
    }

    async fn check_once(&self) -> Result<State, Fault> {
        let url = std::env::var("XAU_EGRESS_TRACE_URL").unwrap_or_else(|_| TRACE.to_string());
        let transport = |e: String| Fault::Transport(e);
        let proxy = reqwest::Proxy::all(self.proxy.as_deref().ok_or_else(|| transport("sem proxy".into()))?)
            .map_err(|e| transport(format!("proxy inválido: {e}")))?;
        let via = reqwest::Client::builder().no_proxy().proxy(proxy).timeout(Duration::from_secs(15)).build().map_err(|e| transport(e.to_string()))?;
        let body = via
            .get(&url)
            .send()
            .await
            .map_err(|e| transport(format!("o proxy não respondeu: {e}")))?
            .text()
            .await
            .map_err(|e| transport(e.to_string()))?;
        let trace = parse_trace(&body);
        // The same request without the proxy: it must not come out of the same address.
        let direct = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(6)).build().map_err(|e| transport(e.to_string()))?;
        let direct_ip = match direct.get(&url).send().await {
            Ok(r) => r.text().await.ok().map(|b| parse_trace(&b)).and_then(|t| t.get("ip").cloned()),
            Err(_) => None,
        };
        judge(&trace, direct_ip.as_deref()).map_err(Fault::Verdict)?;
        Ok(State {
            ok: true,
            ip: trace.get("ip").cloned().unwrap_or_default(),
            colo: trace.get("colo").cloned().unwrap_or_default(),
            warp: trace.get("warp").cloned().unwrap_or_default(),
            detail: String::new(),
            last_ok: Some(Instant::now()),
        })
    }

    /// Keep the check going for the life of the server. Often (every few seconds) because an idle
    /// WARP tunnel can fall asleep and then take a while to wake: the checks are also its keep-alive.
    ///
    /// A verdict that the exit is not Cloudflare's closes the door at once. A proxy that does not
    /// answer closes it only if it has not answered for a while (`GRACE`): a short stumble must not
    /// make the site flap, and nothing leaks meanwhile, since every client goes through that proxy
    /// and fails when it is down, never around it.
    pub async fn watch(self: Arc<Self>) {
        if self.proxy.is_none() {
            return;
        }
        loop {
            let wait = match self.check_once().await {
                Ok(s) => {
                    let first = !self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).ok;
                    if first {
                        tracing::info!("saída pela Cloudflare confirmada (warp={}, colo={})", s.warp, s.colo);
                    }
                    *self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = s;
                    CHECK_EVERY
                }
                Err(fault) => {
                    let mut st = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let (detail, keeps_trust) = match fault {
                        Fault::Transport(e) => (e, still_trusted(st.last_ok, Instant::now())),
                        Fault::Verdict(e) => (e, false),
                    };
                    tracing::warn!("saída pela Cloudflare: {detail}{}", if keeps_trust { " (ainda dentro da tolerância)" } else { "" });
                    st.detail = detail;
                    if !keeps_trust {
                        st.ok = false;
                    }
                    RETRY_EVERY
                }
            };
            tokio::time::sleep(wait).await;
        }
    }
}

/// Why a check failed: the proxy did not answer (`Transport`), or it answered and the exit is not
/// Cloudflare's (`Verdict`).
enum Fault {
    Transport(String),
    Verdict(String),
}

const CHECK_EVERY: Duration = Duration::from_secs(10);
const RETRY_EVERY: Duration = Duration::from_secs(3);
/// How long a proxy that stopped answering keeps the confirmation it earned.
const GRACE: Duration = Duration::from_secs(45);

fn still_trusted(last_ok: Option<Instant>, now: Instant) -> bool {
    last_ok.is_some_and(|t| now.saturating_duration_since(t) < GRACE)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn trace(s: &str) -> HashMap<String, String> {
        parse_trace(s)
    }

    #[test]
    fn the_trace_page_is_read_as_key_value_lines() {
        let t = trace("fl=1\nip=104.28.1.2\ncolo=GRU\nwarp=on\ngateway=off\n");
        assert_eq!(t["colo"], "GRU");
        assert_eq!(t["warp"], "on");
    }

    #[test]
    fn only_warp_or_gateway_counts_as_cloudflare() {
        assert!(judge(&trace("ip=1.1.1.1\nwarp=on\ngateway=off"), Some("9.9.9.9")).is_ok());
        assert!(judge(&trace("ip=1.1.1.1\nwarp=plus"), None).is_ok());
        assert!(judge(&trace("ip=1.1.1.1\nwarp=off\ngateway=on"), None).is_ok());
        assert!(judge(&trace("ip=1.1.1.1\nwarp=off\ngateway=off"), None).is_err(), "a plain datacenter address");
        assert!(judge(&trace("ip=1.1.1.1"), None).is_err(), "no field at all");
    }

    #[test]
    fn a_short_stumble_keeps_the_confirmation_a_long_silence_does_not() {
        let now = Instant::now();
        assert!(still_trusted(Some(now), now + Duration::from_secs(20)));
        assert!(!still_trusted(Some(now), now + GRACE + Duration::from_secs(1)));
        assert!(!still_trusted(None, now), "never confirmed: never trusted");
    }

    #[test]
    fn a_proxy_that_changes_nothing_is_caught() {
        assert!(judge(&trace("ip=5.5.5.5\nwarp=on"), Some("5.5.5.5")).is_err());
    }

    #[test]
    fn a_server_without_a_proxy_refuses_until_it_is_told_otherwise() {
        let e = Egress { proxy: None, required: true, state: Mutex::default() };
        assert!(e.require().is_err());
        let open = Egress { proxy: None, required: false, state: Mutex::default() };
        assert!(open.require().is_ok(), "a computer's server goes out as it always did");
        let waiting = Egress { proxy: Some("socks5h://127.0.0.1:40000".into()), required: true, state: Mutex::default() };
        assert!(waiting.require().is_err(), "not confirmed yet is not allowed yet");
        waiting.state.lock().unwrap().ok = true;
        assert!(waiting.require().is_ok());
    }
}
