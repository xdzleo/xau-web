//! The sign-in page's browser, for a server with no screen: headless Chromium
//! driven over the DevTools protocol.
//!
//! It does what the WebView2 window does on Windows (see `webhost`), for the
//! same callers: the page runs at the size of the site's screen, its frames
//! are pushed back through the screencast, the person's touches and keys are
//! replayed as real input, and the core's script and headers reach Microsoft's
//! page. One browser process per window, so each sign-in has its own profile
//! folder (cookies, storage) exactly as with WebView2.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;

use crate::webhost::{unhosted_signup, SignInHandle, SignInSpec, WinEvent};

const INLINE_LOGIN: &str = "https://login.live.com/ppsecure/InlineLogin.srf";
/// The name of the function the page calls to reach the server (`Runtime.addBinding`).
const BINDING: &str = "__xauSend";

enum Cmd {
    Cdp { method: String, params: String, reply: oneshot::Sender<Result<String, String>> },
    Eval(String),
    Navigate(String),
    Close,
}

#[derive(Clone)]
pub struct Host {
    next: Arc<AtomicU64>,
    windows: Arc<Mutex<HashMap<u64, UnboundedSender<Cmd>>>>,
}

impl Host {
    pub fn new() -> Self {
        Self { next: Arc::new(AtomicU64::new(1)), windows: Arc::default() }
    }

    fn send(&self, win: u64, cmd: Cmd) -> Result<(), String> {
        let windows = self.windows.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        windows.get(&win).ok_or_else(|| "a janela fechou".to_string())?.send(cmd).map_err(|_| "a janela fechou".to_string())
    }

    pub fn open_sign_in(&self, spec: SignInSpec) -> Result<SignInHandle, String> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let (evt_tx, evt_rx) = unbounded_channel();
        self.windows.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(id, cmd_tx);
        let windows = self.windows.clone();
        tokio::spawn(async move {
            if let Err(e) = run(spec, cmd_rx, evt_tx.clone()).await {
                tracing::warn!("sign-in browser: {e}");
                let _ = evt_tx.send(WinEvent::Failed(e));
            }
            windows.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&id);
        });
        Ok((id, evt_rx))
    }

    pub fn eval(&self, win: u64, js: String) {
        let _ = self.send(win, Cmd::Eval(js));
    }

    pub fn navigate(&self, win: u64, url: String) {
        let _ = self.send(win, Cmd::Navigate(url));
    }

    pub fn load(&self, win: u64, url: String) {
        let _ = self.send(win, Cmd::Navigate(url));
    }

    pub fn close(&self, win: u64) {
        let _ = self.send(win, Cmd::Close);
    }

    pub async fn cdp(&self, win: u64, method: &str, params: &str) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        self.send(win, Cmd::Cdp { method: method.into(), params: params.into(), reply })?;
        match tokio::time::timeout(Duration::from_secs(20), rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("a janela fechou".into()),
            Err(_) => Err("a página não respondeu".into()),
        }
    }
}

/// The browser to run: `XAU_CHROMIUM`, else the usual places.
fn chromium_path() -> Result<String, String> {
    if let Ok(p) = std::env::var("XAU_CHROMIUM") {
        return Ok(p);
    }
    let candidates: &[&str] = if cfg!(windows) {
        &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        ]
    } else {
        &["/usr/bin/chromium", "/usr/bin/chromium-browser", "/usr/bin/google-chrome", "/usr/bin/google-chrome-stable"]
    };
    candidates
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(|p| (*p).to_string())
        .ok_or_else(|| "não achei o Chromium (defina XAU_CHROMIUM)".to_string())
}

/// A request the browser holds for us (`Fetch.requestPaused`): `(request id, url, headers)`.
fn paused(params: &Value) -> Option<(String, String, Vec<(String, String)>)> {
    let id = params["requestId"].as_str()?.to_string();
    let url = params["request"]["url"].as_str()?.to_string();
    let headers = params["request"]["headers"]
        .as_object()
        .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    Some((id, url, headers))
}

async fn run(spec: SignInSpec, mut cmds: UnboundedReceiver<Cmd>, events: UnboundedSender<WinEvent>) -> Result<(), String> {
    let exe = chromium_path()?;
    let mut args: Vec<String> = vec![
        "--headless=new".into(),
        "--remote-debugging-port=0".into(),
        format!("--user-data-dir={}", spec.profile_dir.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--disable-gpu".into(),
        "--disable-dev-shm-usage".into(),
        "--hide-scrollbars".into(),
        "--mute-audio".into(),
        // A server's browser talks to the page it was asked to open and to nothing else: no Google
        // services, updates, sync or telemetry.
        "--disable-background-networking".into(),
        "--disable-component-update".into(),
        "--disable-sync".into(),
        "--disable-default-apps".into(),
        "--disable-extensions".into(),
        "--disable-domain-reliability".into(),
        "--disable-client-side-phishing-detection".into(),
        "--no-pings".into(),
        "--metrics-recording-only".into(),
        "--safebrowsing-disable-auto-update".into(),
        // The page is shown inside the site: nothing may slow it down for being out of sight.
        "--disable-features=CalculateNativeWinOcclusion,OptimizationHints,Translate,MediaRouter,AutofillServerCommunication".into(),
        "--disable-background-timer-throttling".into(),
        "--disable-renderer-backgrounding".into(),
        "--disable-backgrounding-occluded-windows".into(),
    ];
    // Everything the browser sends leaves through the proxy (Cloudflare's WARP), names included: it
    // resolves nothing itself (the loopback, where the server talks to it, is the only exception).
    if let Ok(p) = std::env::var("XAU_EGRESS_PROXY") {
        let p = p.trim().replace("socks5h://", "socks5://");
        if !p.is_empty() {
            args.push(format!("--proxy-server={p}"));
            args.push("--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE 127.0.0.1 , EXCLUDE localhost".into());
        }
    }
    // A container runs as root, where Chromium's own sandbox cannot start.
    if std::env::var_os("XAU_CHROMIUM_NO_SANDBOX").is_some() {
        args.push("--no-sandbox".into());
    }
    args.push("about:blank".into());

    // The browser says where it listens in a file of its profile folder (the same on every system;
    // its own announcement on stderr is not written everywhere).
    let _ = std::fs::create_dir_all(&spec.profile_dir);
    let port_file = spec.profile_dir.join("DevToolsActivePort");
    let _ = std::fs::remove_file(&port_file);

    let mut child = tokio::process::Command::new(&exe)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("não consegui abrir o Chromium ({exe}): {e}"))?;

    // "PORT\n/devtools/browser/ID"
    let started = std::time::Instant::now();
    let mut launcher_gone = false;
    let ws_url = loop {
        if let Ok(text) = std::fs::read_to_string(&port_file) {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                break format!("ws://127.0.0.1:{}{}", port.trim(), path.trim());
            }
        }
        // A launcher that hands over to another process exits 0 (Chrome on Windows): not a failure.
        if let Ok(Some(status)) = child.try_wait() {
            if !status.success() {
                return Err(format!("o Chromium fechou antes de ficar pronto ({status})"));
            }
            launcher_gone = true;
        }
        if started.elapsed() > Duration::from_secs(30) {
            return Err("o Chromium não respondeu a tempo".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let (ws, _) = tokio_tungstenite::connect_async(&ws_url).await.map_err(|e| format!("DevTools: {e}"))?;
    let (mut sink, mut stream) = ws.split();

    let next_id = AtomicU64::new(1);
    let mut waiting: HashMap<u64, oneshot::Sender<Result<String, String>>> = HashMap::new();
    let mut session: Option<String> = None;

    // A command to the browser or, with a session, to the page.
    macro_rules! send_raw {
        ($method:expr, $params:expr, $reply:expr) => {{
            let id = next_id.fetch_add(1, Ordering::SeqCst);
            let mut msg = json!({ "id": id, "method": $method, "params": $params });
            if let Some(s) = &session {
                msg["sessionId"] = json!(s);
            }
            if let Some(r) = $reply {
                waiting.insert(id, r);
            }
            sink.send(Message::Text(msg.to_string())).await.map_err(|e| e.to_string())?;
        }};
    }

    // Find the page that opens with the browser and attach to it (flat: one socket for everything).
    let (tx, rx) = oneshot::channel();
    send_raw!("Target.getTargets", json!({}), Some(tx));
    let mut ready: Option<oneshot::Receiver<Result<String, String>>> = Some(rx);
    let mut attach_wait: Option<oneshot::Receiver<Result<String, String>>> = None;
    let mut setup_done = false;

    let finished = Arc::new(Mutex::new(false));
    let mut last_url = String::new();
    let mut setup_pending: Vec<oneshot::Receiver<Result<String, String>>> = Vec::new();
    let mut queued: Vec<Cmd> = Vec::new();

    loop {
        // Setup is a short sequence of replies; each stage unlocks the next.
        if let Some(r) = ready.as_mut() {
            if let Ok(res) = r.try_recv() {
                ready = None;
                let targets: Value = serde_json::from_str(&res?).map_err(|e| e.to_string())?;
                let target = targets["targetInfos"]
                    .as_array()
                    .and_then(|a| a.iter().find(|t| t["type"] == "page"))
                    .and_then(|t| t["targetId"].as_str())
                    .ok_or("o Chromium não abriu uma página")?
                    .to_string();
                let (tx, rx) = oneshot::channel();
                send_raw!("Target.attachToTarget", json!({ "targetId": target, "flatten": true }), Some(tx));
                attach_wait = Some(rx);
            }
        }
        if let Some(r) = attach_wait.as_mut() {
            if let Ok(res) = r.try_recv() {
                attach_wait = None;
                let v: Value = serde_json::from_str(&res?).map_err(|e| e.to_string())?;
                session = Some(v["sessionId"].as_str().ok_or("sem sessão da página")?.to_string());

                // The page: events, the bridge the core's script reports through, and its scripts.
                let bridge = format!(
                    "window.{b} = {{ postMessage: function (m) {{ window.{BINDING}(String(m)); }} }};",
                    b = spec.bridge
                );
                // A link that asks for a new window opens in this page, as in the app's WebView.
                let popups = "window.open = function (u) { if (u) location.href = String(u); return window; }; \
                              document.addEventListener('click', function (e) { var a = e.target && e.target.closest && e.target.closest('a[target]'); \
                              if (a && a.target !== '_self') a.target = '_self'; }, true);";
                let script = format!("(function () {{ {bridge} {popups} }})();\n{}", spec.init_script);
                let mut patterns = vec![json!({ "urlPattern": format!("{INLINE_LOGIN}*"), "resourceType": "Document" })];
                if !spec.side {
                    patterns.push(json!({ "urlPattern": "https://signup.live.com/*", "resourceType": "Document" }));
                }
                for (m, p) in [
                    ("Page.enable", json!({})),
                    ("Runtime.enable", json!({})),
                    ("Network.enable", json!({})),
                    ("Runtime.addBinding", json!({ "name": BINDING })),
                    ("Page.addScriptToEvaluateOnNewDocument", json!({ "source": script })),
                    ("Fetch.enable", json!({ "patterns": patterns })),
                ] {
                    let (tx, rx) = oneshot::channel();
                    send_raw!(m, p, Some(tx));
                    setup_pending.push(rx);
                }
            }
        }
        if !setup_done && session.is_some() && attach_wait.is_none() && !setup_pending.is_empty() {
            if setup_pending.iter_mut().all(|r| !matches!(r.try_recv(), Err(oneshot::error::TryRecvError::Empty))) {
                setup_done = true;
                setup_pending.clear();
                // What was asked while the browser started (emulation, the first page, the screencast).
                for c in std::mem::take(&mut queued) {
                    handle_cmd(c, &mut waiting, &next_id, &mut sink, &session).await?;
                }
            }
        }

        tokio::select! {
            msg = stream.next() => {
                let Some(Ok(msg)) = msg else { break };
                let Message::Text(text) = msg else { continue };
                let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                if let Some(id) = v["id"].as_u64() {
                    if let Some(reply) = waiting.remove(&id) {
                        let _ = reply.send(match v.get("error") {
                            Some(e) => Err(e["message"].as_str().unwrap_or("erro do navegador").to_string()),
                            None => Ok(v["result"].to_string()),
                        });
                    }
                    continue;
                }
                let params = &v["params"];
                match v["method"].as_str().unwrap_or("") {
                    "Runtime.bindingCalled" if params["name"] == BINDING => {
                        if let Some(p) = params["payload"].as_str() {
                            let _ = events.send(WinEvent::Message(p.to_string()));
                        }
                    }
                    "Page.frameNavigated" => {
                        if params["frame"]["parentId"].is_null() {
                            last_url = params["frame"]["url"].as_str().unwrap_or("").to_string();
                        }
                    }
                    "Page.loadEventFired" => {
                        let _ = events.send(WinEvent::PageFinished(last_url.clone()));
                    }
                    "Page.screencastFrame" => {
                        if let Some(n) = params["sessionId"].as_i64() {
                            send_raw!("Page.screencastFrameAck", json!({ "sessionId": n }), None::<oneshot::Sender<Result<String, String>>>);
                        }
                        if let (Some(frames), Ok(jpeg)) = (
                            spec.frames.as_ref(),
                            base64::engine::general_purpose::STANDARD.decode(params["data"].as_str().unwrap_or("")),
                        ) {
                            frames.send_replace(Some(bytes::Bytes::from(jpeg)));
                        }
                    }
                    "Fetch.requestPaused" => {
                        let Some((rid, url, headers)) = paused(params) else { continue };
                        if !spec.side && unhosted_signup(&url).is_some() {
                            // "Create one!" in the Windows-hosted mode: the sign-in opens the plain one.
                            let plain = unhosted_signup(&url).unwrap_or_default();
                            send_raw!("Fetch.failRequest", json!({ "requestId": rid, "errorReason": "BlockedByClient" }), None::<oneshot::Sender<Result<String, String>>>);
                            let _ = events.send(WinEvent::Leave { url: plain, to_signup: true });
                        } else if spec.side && url.starts_with(INLINE_LOGIN) {
                            // The way back from the sign-up.
                            send_raw!("Fetch.failRequest", json!({ "requestId": rid, "errorReason": "BlockedByClient" }), None::<oneshot::Sender<Result<String, String>>>);
                            let _ = events.send(WinEvent::Leave { url, to_signup: false });
                        } else if !spec.side && url.starts_with(INLINE_LOGIN) {
                            // Microsoft serves the page the app shows only to a request carrying the core's headers.
                            let mut merged: Vec<Value> = headers
                                .iter()
                                .filter(|(k, _)| !spec.headers.iter().any(|(h, _)| h.eq_ignore_ascii_case(k)))
                                .map(|(k, v)| json!({ "name": k, "value": v }))
                                .collect();
                            merged.extend(spec.headers.iter().map(|(k, v)| json!({ "name": k, "value": v })));
                            send_raw!("Fetch.continueRequest", json!({ "requestId": rid, "headers": merged }), None::<oneshot::Sender<Result<String, String>>>);
                        } else {
                            send_raw!("Fetch.continueRequest", json!({ "requestId": rid }), None::<oneshot::Sender<Result<String, String>>>);
                        }
                    }
                    "Inspector.detached" | "Target.targetCrashed" => break,
                    _ => {}
                }
            }
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { break };
                if matches!(cmd, Cmd::Close) {
                    *finished.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = true;
                    break;
                }
                if setup_done {
                    handle_cmd(cmd, &mut waiting, &next_id, &mut sink, &session).await?;
                } else {
                    queued.push(cmd);
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(20)), if !setup_done => {}
            status = child.wait(), if !launcher_gone => {
                tracing::debug!("chromium exited: {status:?}");
                // A launcher that handed over to another process: the socket tells when it is over.
                if launcher_gone_ok(&status) { launcher_gone = true; } else { break; }
            }
        }
    }

    // Closed by us: ask the browser to quit (it may not be the process we started), then make sure.
    let _ = tokio::time::timeout(Duration::from_millis(500), sink.send(Message::Text(json!({ "id": next_id.fetch_add(1, Ordering::SeqCst), "method": "Browser.close" }).to_string()))).await;
    let by_us = *finished.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = child.start_kill();
    // Closed by us (done, cancelled): nobody is told, they asked for it.
    if !by_us {
        let _ = events.send(WinEvent::Closed);
    }
    Ok(())
}

async fn handle_cmd<S>(
    cmd: Cmd,
    waiting: &mut HashMap<u64, oneshot::Sender<Result<String, String>>>,
    next_id: &AtomicU64,
    sink: &mut S,
    session: &Option<String>,
) -> Result<(), String>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let (method, params, reply) = match cmd {
        Cmd::Cdp { method, params, reply } => (method, serde_json::from_str(&params).unwrap_or(json!({})), Some(reply)),
        Cmd::Eval(js) => ("Runtime.evaluate".to_string(), json!({ "expression": js }), None),
        Cmd::Navigate(url) => ("Page.navigate".to_string(), json!({ "url": url }), None),
        Cmd::Close => return Ok(()),
    };
    let id = next_id.fetch_add(1, Ordering::SeqCst);
    let mut msg = json!({ "id": id, "method": method, "params": params });
    if let Some(s) = session {
        msg["sessionId"] = json!(s);
    }
    if let Some(r) = reply {
        waiting.insert(id, r);
    }
    sink.send(Message::Text(msg.to_string())).await.map_err(|e| e.to_string())
}

/// A browser process that ends with success while the DevTools socket is still up has handed over.
fn launcher_gone_ok(status: &std::io::Result<std::process::ExitStatus>) -> bool {
    status.as_ref().is_ok_and(std::process::ExitStatus::success)
}