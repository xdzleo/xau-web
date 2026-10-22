//! The sign-in, run from the server.
//!
//! It is the same flow as the Android `LoginScreen`: the core says which
//! page to load and what each message from it means (`windows_login_*`),
//! and this module is the host that loads the page in a real window and
//! carries the core's answers back to it. The browser UI only sees a
//! status it can poll.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use xau_api::dispatch;

use crate::profiles;
use crate::webhost::{Host, SignInSpec, WinEvent};

#[derive(Clone, Debug)]
pub enum Status {
    /// The window is open, the user is typing.
    Waiting,
    /// Microsoft handed over its answer; the core is exchanging it.
    Working,
    Done { accounts: Value, warning: Option<String> },
    Failed(String),
    Cancelled,
}

impl Status {
    pub fn finished(&self) -> bool {
        !matches!(self, Status::Waiting | Status::Working)
    }

    pub fn to_json(&self) -> Value {
        match self {
            Status::Waiting => json!({ "state": "waiting" }),
            Status::Working => json!({ "state": "working" }),
            Status::Done { accounts, warning } => {
                json!({ "state": "done", "accounts": accounts, "warning": warning })
            }
            Status::Failed(e) => json!({ "state": "failed", "error": e }),
            Status::Cancelled => json!({ "state": "cancelled" }),
        }
    }
}

#[derive(Default)]
pub struct Logins {
    map: Mutex<HashMap<String, Status>>,
    /// The window of the sign-in in progress, to close it on cancel.
    window: Mutex<Option<(String, u64)>>,
    /// The size (CSS px, density) the page was last given, to resize it only when the site's screen changes.
    view: Mutex<(u32, u32, u32)>,
    /// The page's live picture (JPEG frames pushed by the engine), for the sign-in in progress.
    frames: Mutex<Option<(String, tokio::sync::watch::Receiver<Option<bytes::Bytes>>)>>,
    /// The window shown changed (sign-in <-> sign-up): live pictures reconnect to the new one.
    pub swapped: tokio::sync::Notify,
    /// Windows opened beside the sign-in (the sign-up), closed with it.
    side: Mutex<Vec<u64>>,
}

impl Logins {
    fn set(&self, id: &str, s: Status) {
        self.map.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(id.to_string(), s);
    }

    pub fn get(&self, id: &str) -> Option<Status> {
        self.map.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(id).cloned()
    }

    fn busy(&self) -> bool {
        self.map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|s| !s.finished())
    }

    /// The user cancelled from the page: close the window.
    pub async fn cancel(&self, host: &Host, id: &str) {
        let win = {
            let mut slot = self.window.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match &*slot {
                Some((wid, _)) if wid == id => slot.take(),
                _ => None,
            }
        };
        if let Some((_, win)) = win {
            // Closing the window from here tells nobody, so the sign-in is ended here.
            let _ = dispatch("windows_login_cancel", &json!({})).await;
            self.set(id, Status::Cancelled);
            host.close(win);
        }
        for w in std::mem::take(&mut *self.side.lock().unwrap_or_else(std::sync::PoisonError::into_inner)) {
            host.close(w);
        }
    }
}

/// One picture of the sign-in page, and the size of the page in CSS pixels
/// (what the pointer coordinates of [`Logins::input`] are in).
pub struct Frame {
    pub jpeg: Vec<u8>,
    /// A text field has the focus: the site shows the keyboard.
    pub editing: bool,
}

impl Logins {
    fn window_of(&self, id: &str) -> Result<u64, String> {
        match &*self.window.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
            Some((wid, win)) if wid == id => Ok(*win),
            _ => Err("este login já terminou".into()),
        }
    }

    /// Give the page the size of the site's screen for it (CSS px) and the person's display density,
    /// so there are no borders and a touch lands where it is drawn.
    async fn set_view(&self, host: &Host, win: u64, width: u32, height: u32, density: u32) -> Result<(), String> {
        let want = (width.clamp(240, 1200), height.clamp(300, 2000), density.clamp(100, 300));
        let changed = {
            let mut v = self.view.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let changed = *v != want;
            *v = want;
            changed
        };
        if changed {
            let p = json!({ "width": want.0, "height": want.1, "deviceScaleFactor": f64::from(want.2) / 100.0, "mobile": true });
            host.cdp(win, "Emulation.setDeviceMetricsOverride", &p.to_string()).await?;
        }
        Ok(())
    }

    /// Whether a text field has the focus (the site then shows the keyboard).
    async fn editing(host: &Host, win: u64) -> bool {
        let js = r#"(function(){var a=document.activeElement;if(!a)return false;if(a.isContentEditable)return true;var t=(a.tagName||'').toUpperCase();if(t==='TEXTAREA')return true;return t==='INPUT'&&!/^(button|submit|checkbox|radio|image|reset|file)$/i.test(a.type||'');})()"#;
        let params = json!({ "expression": js, "returnByValue": true }).to_string();
        host.cdp(win, "Runtime.evaluate", &params)
            .await
            .ok()
            .and_then(|e| serde_json::from_str::<Value>(&e).ok())
            .is_some_and(|v| v["result"]["value"].as_bool() == Some(true))
    }

    /// The live picture of the page: every frame the engine draws, pushed as it comes.
    pub async fn stream(
        &self,
        host: &Host,
        id: &str,
        width: u32,
        height: u32,
        density: u32,
    ) -> Result<tokio::sync::watch::Receiver<Option<bytes::Bytes>>, String> {
        let win = self.window_of(id)?;
        let rx = match &*self.frames.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
            Some((fid, rx)) if fid == id => rx.clone(),
            _ => return Err("este login não tem imagem ao vivo".into()),
        };
        self.set_view(host, win, width, height, density).await?;
        Ok(rx)
    }

    /// The page now, at the display's own density (the screencast's frames are in CSS pixels):
    /// sent once the page holds still, so what is read and typed is sharp.
    pub async fn sharp_frame(&self, host: &Host, id: &str) -> Result<Vec<u8>, String> {
        use base64::Engine;
        let win = self.window_of(id)?;
        let shot = host.cdp(win, "Page.captureScreenshot", r#"{"format":"jpeg","quality":85}"#).await?;
        let shot: Value = serde_json::from_str(&shot).map_err(|e| e.to_string())?;
        base64::engine::general_purpose::STANDARD.decode(shot["data"].as_str().unwrap_or("")).map_err(|e| e.to_string())
    }

    /// One picture of the page (the fallback when the live picture cannot be shown).
    pub async fn frame(&self, host: &Host, id: &str, width: u32, height: u32, density: u32) -> Result<Frame, String> {
        use base64::Engine;
        let win = self.window_of(id)?;
        self.set_view(host, win, width, height, density).await?;
        let (shot, editing) = tokio::join!(
            host.cdp(win, "Page.captureScreenshot", r#"{"format":"jpeg","quality":70}"#),
            Self::editing(host, win),
        );
        let shot: Value = serde_json::from_str(&shot?).map_err(|e| e.to_string())?;
        let jpeg = base64::engine::general_purpose::STANDARD
            .decode(shot["data"].as_str().unwrap_or(""))
            .map_err(|e| e.to_string())?;
        Ok(Frame { jpeg, editing })
    }
    /// A pointer or key event from the person at the page, replayed in the
    /// sign-in page as a real (trusted) input event. Only a fixed set of
    /// fields is read, each checked, and the call is built here, never
    /// taken from the request.
    pub async fn input(&self, host: &Host, id: &str, ev: &Value) -> Result<Option<bool>, String> {
        if !self.dispatch_input(host, id, ev).await? {
            return Ok(None);
        }
        Ok(Some(self.editing_after_input(host, id).await))
    }

    /// Whether a text field has the focus, a moment after an input that may have moved it.
    pub async fn editing_after_input(&self, host: &Host, id: &str) -> bool {
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        match self.window_of(id) {
            Ok(win) => Self::editing(host, win).await,
            Err(_) => false,
        }
    }

    /// Replay one input; true when it may have moved the focus (a tap, a click, Tab or Enter).
    pub async fn dispatch_input(&self, host: &Host, id: &str, ev: &Value) -> Result<bool, String> {
        let win = self.window_of(id)?;
        let num = |k: &str| ev.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0);
        let text = |k: &str, max: usize| ev.get(k).and_then(Value::as_str).map(|s| s.chars().take(max).collect::<String>());
        let (method, params) = match ev["kind"].as_str() {
            Some("mouse") => {
                let ty = match ev["type"].as_str() {
                    Some(t @ ("mousePressed" | "mouseReleased" | "mouseMoved" | "mouseWheel")) => t,
                    _ => return Err("evento inválido".into()),
                };
                let button = match ev["button"].as_str() {
                    Some(b @ ("left" | "middle" | "right")) => b,
                    _ => "none",
                };
                (
                    "Input.dispatchMouseEvent",
                    json!({
                        "type": ty, "x": num("x").clamp(0.0, 4000.0), "y": num("y").clamp(0.0, 4000.0),
                        "button": button, "clickCount": num("clickCount").clamp(0.0, 3.0) as i64,
                        "deltaX": num("deltaX").clamp(-4000.0, 4000.0), "deltaY": num("deltaY").clamp(-4000.0, 4000.0),
                    }),
                )
            }
            Some("touch") => {
                let ty = match ev["type"].as_str() {
                    Some(t @ ("touchStart" | "touchMove" | "touchEnd" | "touchCancel")) => t,
                    _ => return Err("evento inválido".into()),
                };
                let points = if ty == "touchEnd" || ty == "touchCancel" {
                    json!([])
                } else {
                    json!([{ "x": num("x").clamp(0.0, 4000.0), "y": num("y").clamp(0.0, 4000.0) }])
                };
                ("Input.dispatchTouchEvent", json!({ "type": ty, "touchPoints": points }))
            }
            Some("key") => {
                let ty = match ev["type"].as_str() {
                    Some(t @ ("keyDown" | "keyUp" | "rawKeyDown")) => t,
                    _ => return Err("evento inválido".into()),
                };
                let mut p = json!({
                    "type": ty,
                    "key": text("key", 32).unwrap_or_default(),
                    "code": text("code", 32).unwrap_or_default(),
                    "windowsVirtualKeyCode": num("vk").clamp(0.0, 255.0) as i64,
                    "modifiers": num("modifiers").clamp(0.0, 15.0) as i64,
                });
                if let Some(t) = text("text", 8).filter(|t| !t.is_empty()) {
                    p["text"] = json!(t);
                    p["unmodifiedText"] = json!(t);
                }
                ("Input.dispatchKeyEvent", p)
            }
            Some("text") => ("Input.insertText", json!({ "text": text("text", 4096).unwrap_or_default() })),
            _ => return Err("evento inválido".into()),
        };
        host.cdp(win, method, &params.to_string()).await?;
        // After a tap, a click or Tab/Enter the focus may have moved.
        Ok(matches!(ev["type"].as_str(), Some("touchEnd" | "mouseReleased"))
            || matches!(ev["key"].as_str(), Some("Tab" | "Enter")) && ev["type"].as_str() == Some("keyUp"))
    }
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Open the sign-in window and drive it in the background. Returns the id
/// to poll with [`Logins::get`].
pub async fn start(host: Host, logins: Arc<Logins>) -> Result<String, String> {
    // The window is modal in spirit: two at once would race over the
    // core's single pending sign-in.
    if logins.busy() {
        return Err("já existe um login em andamento: termine ou feche a janela".into());
    }
    let page = dispatch("windows_login_start", &json!({})).await.map_err(|e| e.to_string())?;

    let headers: Vec<(String, String)> = page["headers"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let spec = SignInSpec {
        title: "Entrar na conta Microsoft".into(),
        url: s(&page, "url"),
        headers,
        user_agent: s(&page, "user_agent"),
        init_script: s(&page, "init_script"),
        bridge: s(&page, "bridge"),
        profile_dir: profiles::new_signin(),
        // The page is shown inside the site; `XAU_LOGIN_WINDOW=1` brings the real window back.
        hidden: std::env::var_os("XAU_LOGIN_WINDOW").is_none(),
        frames: None,
        side: false,
    };
    let mut spec = spec;
    let frames_rx = if spec.hidden {
        let (tx, rx) = tokio::sync::watch::channel(None);
        spec.frames = Some(tx);
        Some(rx)
    } else {
        None
    };
    let profile_dir = spec.profile_dir.clone();
    let spec_hidden = spec.hidden;
    let (first_url, first_ua) = (spec.url.clone(), spec.user_agent.clone());
    let finish_script = s(&page, "finish_script");
    let finish_prefix = s(&page, "finish_url_prefix");

    let (win, mut rx) = match host.open_sign_in(spec) {
        Ok(h) => h,
        Err(e) => {
            let _ = dispatch("windows_login_cancel", &json!({})).await;
            return Err(e);
        }
    };

    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
    logins.set(&id, Status::Waiting);
    // Under Microsoft's page, the app's own dark, as in the app's WebView (it does not fill the screen).
    if spec_hidden {
        // The app signs in inside a phone's WebView, and Microsoft answers with the phone's
        // page: so the page here is a phone's too (size, density, touch) before it loads.
        // What the app's WebView reports about itself (the page reads it through `navigator.userAgentData`).
        let identity = json!({
            "userAgent": first_ua,
            "platform": "Linux aarch64",
            "acceptLanguage": "pt-BR,en-US",
            "userAgentMetadata": {
                "brands": [{"brand": "Android WebView", "version": "153"}, {"brand": "Not_A Brand", "version": "8"}, {"brand": "Chromium", "version": "153"}],
                "fullVersion": "153.0.0.0", "platform": "Android", "platformVersion": "15.0.0", "architecture": "", "model": "moto g34 5G", "mobile": true
            }
        })
        .to_string();
        for (method, params) in [
            ("Emulation.setUserAgentOverride", identity.as_str()),
            ("Emulation.setDeviceMetricsOverride", r#"{"width":412,"height":860,"deviceScaleFactor":1.75,"mobile":true}"#),
            ("Emulation.setTouchEmulationEnabled", r#"{"enabled":true,"maxTouchPoints":5}"#),
            ("Emulation.setDefaultBackgroundColorOverride", r#"{"color":{"r":10,"g":12,"b":10,"a":1}}"#),
        ] {
            if let Err(e) = host.cdp(win, method, params).await {
                tracing::warn!("sign-in emulation {method}: {e}");
            }
        }
        // The core's headers are added to this request by the window itself (see `webhost`).
        host.load(win, first_url, Vec::new());
        // The engine pushes a frame each time the page draws (and none while it is still).
        if let Err(e) = host.cdp(win, "Page.startScreencast", &screencast_params()).await {
            tracing::warn!("sign-in screencast: {e}");
        }
    }    *logins.window.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((id.clone(), win));
    *logins.frames.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = frames_rx.map(|rx| (id.clone(), rx));
    *logins.view.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = (412, 860, 175);

    let task_id = id.clone();
    tokio::spawn(async move {
        let mut finished = false;
        while let Some(ev) = rx.recv().await {
            if finished {
                continue;
            }
            match ev {
                WinEvent::PageFinished(url) => {
                    tracing::debug!("sign-in page loaded: {url}");
                    if !finish_prefix.is_empty() && url.starts_with(&finish_prefix) {
                        logins.set(&task_id, Status::Working);
                        host.eval(win, finish_script.clone());
                    }
                }
                WinEvent::Message(body) => match {
                    tracing::debug!("sign-in page says: {}", body.chars().take(400).collect::<String>());
                    dispatch("windows_login_message", &json!({ "body": body })).await
                } {
                    Ok(v) => match v["action"].as_str() {
                        Some("eval") => host.eval(win, s(&v, "script")),
                        Some("open") => {
                            // Microsoft wants one more page, and it must be visible.
                            logins.set(&task_id, Status::Waiting);
                            host.navigate(win, s(&v, "url"));
                        }
                        Some("done") => {
                            finished = true;
                            let accounts = v["accounts"].clone();
                            // The sign-in's profile becomes the account's.
                            if let Some(xuid) = accounts
                                .as_array()
                                .and_then(|a| a.iter().find(|r| r["active"].as_bool() == Some(true)))
                                .and_then(|r| r["xuid"].as_str())
                            {
                                profiles::bind(xuid, &profile_dir);
                            }
                            let warning = v["warning"].as_str().map(str::to_string);
                            logins.set(&task_id, Status::Done { accounts, warning });
                            host.close(win);
                        }
                        _ => {}
                    },
                    Err(e) => {
                        finished = true;
                        logins.set(&task_id, Status::Failed(e.to_string()));
                        host.close(win);
                    }
                },
                WinEvent::Leave { url, to_signup: true } => {
                    if let Err(e) = open_signup(&host, &logins, &task_id, win, url).await {
                        tracing::warn!("sign-up: {e}");
                    }
                }
                WinEvent::Leave { .. } => {}
                WinEvent::Closed => {
                    finished = true;
                    let _ = dispatch("windows_login_cancel", &json!({})).await;
                    logins.set(&task_id, Status::Cancelled);
                }
                WinEvent::Failed(e) => {
                    finished = true;
                    let _ = dispatch("windows_login_cancel", &json!({})).await;
                    logins.set(&task_id, Status::Failed(e));
                }
            }
            if finished {
                *logins.window.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            }
        }
    });

    Ok(id)
}

/// Benchmark mode (only with `XAU_WEB_BENCH`): the same hidden window, live picture and input
/// as a sign-in, on a local test page instead of Microsoft's, so measuring registers nothing
/// anywhere. The session id is always `bench`.
pub async fn start_bench(host: Host, logins: Arc<Logins>, url: String, windows_ua: bool) -> Result<String, String> {
    let (tx, frames_rx) = tokio::sync::watch::channel(None);
    let spec = SignInSpec {
        title: "xau-web benchmark".into(),
        url: url.clone(),
        headers: Vec::new(),
        user_agent: if windows_ua {
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; MSAppHost/3.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/70.0.3538.102 Safari/537.36 Edge/18.26100".into()
        } else {
            "Mozilla/5.0 (Linux; Android 15) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Mobile Safari/537.36".into()
        },
        init_script: String::new(),
        bridge: "XauBench".into(),
        profile_dir: profiles::new_signin(),
        hidden: true,
        frames: Some(tx),
        side: false,
    };
    if let Some((_, old)) = logins.window.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
        host.close(old);
    }
    let (win, mut rx) = host.open_sign_in(spec)?;
    for (method, params) in [
        ("Emulation.setDeviceMetricsOverride", r#"{"width":412,"height":860,"deviceScaleFactor":1.75,"mobile":true}"#),
        ("Emulation.setTouchEmulationEnabled", r#"{"enabled":true,"maxTouchPoints":5}"#),
    ] {
        host.cdp(win, method, params).await?;
    }
    host.load(win, url, Vec::new());
    host.cdp(win, "Page.startScreencast", &screencast_params()).await?;
    let id = "bench".to_string();
    logins.set(&id, Status::Waiting);
    *logins.window.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((id.clone(), win));
    *logins.frames.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((id.clone(), frames_rx));
    *logins.view.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = (412, 860, 175);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    Ok(id)
}
/// The engine's screencast: every frame, JPEG at the quality measured best (see the benchmark).
fn screencast_params() -> String {
    let quality = std::env::var("XAU_SCREENCAST_QUALITY").ok().and_then(|q| q.parse::<u8>().ok()).unwrap_or(72).clamp(30, 95);
    json!({ "format": "jpeg", "quality": quality, "everyNthFrame": 1 }).to_string()
}

/// Microsoft's sign-up, in a clean window of its own beside the sign-in (the sign-in's window
/// carries the Windows-hosted state the sign-up would stop on). The site shows it in place of
/// the sign-in; when it heads back to the sign-in (done, or "Sign in"), it closes and the site
/// shows the sign-in again.
async fn open_signup(host: &Host, logins: &Arc<Logins>, id: &str, main_win: u64, url: String) -> Result<(), String> {
    let (tx, side_frames) = tokio::sync::watch::channel(None);
    let spec = SignInSpec {
        title: "Criar conta Microsoft".into(),
        url: url.clone(),
        headers: Vec::new(),
        user_agent: "Mozilla/5.0 (Linux; Android 15; moto g34 5G) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Mobile Safari/537.36".into(),
        init_script: String::new(),
        bridge: "XauSignup".into(),
        profile_dir: profiles::new_signin(),
        hidden: true,
        frames: Some(tx),
        side: true,
    };
    let (side_win, mut events) = host.open_sign_in(spec)?;
    logins.side.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(side_win);
    let (w, h, d) = *logins.view.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let metrics = json!({ "width": w, "height": h, "deviceScaleFactor": f64::from(d) / 100.0, "mobile": true }).to_string();
    host.cdp(side_win, "Emulation.setDeviceMetricsOverride", &metrics).await?;
    host.cdp(side_win, "Emulation.setTouchEmulationEnabled", r#"{"enabled":true,"maxTouchPoints":5}"#).await?;
    host.load(side_win, url, Vec::new());
    host.cdp(side_win, "Page.startScreencast", &screencast_params()).await?;

    let main_frames = logins.frames.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().map(|(_, rx)| rx.clone());
    *logins.window.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((id.to_string(), side_win));
    *logins.frames.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((id.to_string(), side_frames));
    logins.swapped.notify_waiters();
    // Out of sight meanwhile, the sign-in page starts over: it was left mid-way (dimmed, waiting)
    // by the link it followed, and must be clean when the person comes back.
    let _ = host.cdp(main_win, "Page.reload", "{}").await;

    let (host, logins, id) = (host.clone(), logins.clone(), id.to_string());
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            if matches!(ev, WinEvent::Leave { to_signup: false, .. } | WinEvent::Closed | WinEvent::Failed(_)) {
                break;
            }
        }
        host.close(side_win);
        logins.side.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|w| *w != side_win);
        // Back to the sign-in, if it is still going.
        let mut slot = logins.window.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(&*slot, Some((wid, w)) if *wid == id && *w == side_win) {
            *slot = Some((id.clone(), main_win));
            drop(slot);
            *logins.frames.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = main_frames.map(|rx| (id, rx));
            logins.swapped.notify_waiters();
        }
    });
    Ok(())
}