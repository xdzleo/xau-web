//! The HTTP side: the web UI, and the same commands the Android app has.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::signin::{self, Logins};
use crate::webhost::{CheckoutSpec, Host};
use crate::{assets, auth, profiles};

#[derive(Clone)]
pub struct App {
    pub key: Arc<String>,
    pub port: u16,
    pub access: auth::Access,
    pub egress: Arc<crate::egress::Egress>,
    pub host: Host,
    pub logins: Arc<Logins>,
    /// `select` then `store_purchase_form` act on "the active account":
    /// two checkouts at once must not swap it between the calls.
    pub checkout: Arc<tokio::sync::Mutex<()>>,
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/", get(index))
        .route("/assets/*path", get(asset))
        .route("/api/info", get(info))
        .route("/api/call/:method", post(call))
        .route("/api/login/start", post(login_start))
        .route("/api/login/:id", get(login_status))
        .route("/api/login/:id/cancel", post(login_cancel))
        .route("/api/login/:id/frame", get(login_frame))
        .route("/api/login/:id/stream", get(login_stream))
        .route("/api/login/:id/ws", get(login_ws))
        .route("/api/login/:id/input", post(login_input))
        .route("/api/checkout", post(checkout))
        .route("/api/bench/start", post(bench_start))
        .route("/bench-page", get(bench_page))
        .with_state(app)
}

// ------------------------------------------------------------- plumbing

fn secure(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' https: data: blob:; style-src 'self' 'unsafe-inline'; \
             font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn text(status: StatusCode, body: &'static str) -> Response {
    secure((status, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response())
}

/// Every API call: this machine, the key, and (for a POST) this origin.
fn guard(app: &App, h: &HeaderMap, post: bool) -> Result<(), Response> {
    if !app.access.host_ok(h) {
        return Err(text(StatusCode::FORBIDDEN, "host não permitido"));
    }
    if !app.access.cookie_ok(h, app.port, &app.key) {
        return Err(text(StatusCode::UNAUTHORIZED, "abra o link impresso no terminal"));
    }
    if post && !app.access.post_ok(h) {
        return Err(text(StatusCode::FORBIDDEN, "origem não permitida"));
    }
    Ok(())
}

fn json_response(v: Value) -> Response {
    secure(Json(v).into_response())
}

fn ok(v: Value) -> Response {
    json_response(json!({ "ok": v }))
}

fn err(e: impl ToString) -> Response {
    json_response(json!({ "err": e.to_string() }))
}

// --------------------------------------------------------------- pages

/// For the host's health check: that the server is up, and nothing else (no names, no data).
async fn healthz() -> Response {
    secure(([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], "ok").into_response())
}

async fn index(State(app): State<App>, h: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if !app.access.host_ok(&h) {
        return text(StatusCode::FORBIDDEN, "host não permitido");
    }
    // The printed link carries the key once; from then on a cookie does.
    if q.get("key").is_some_and(|k| auth::same(k, &app.key)) {
        // Over https (a public server) the cookie is never sent over plain http.
        let secure_flag = if app.access.scheme(&h) == "https" { "; Secure" } else { "" };
        let cookie = format!(
            "{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=31536000{secure_flag}",
            app.access.cookie_name(app.port),
            app.key
        );
        return secure(
            (StatusCode::FOUND, [(header::LOCATION, "/".to_string()), (header::SET_COOKIE, cookie)]).into_response(),
        );
    }
    if !app.access.cookie_ok(&h, app.port, &app.key) {
        return text(
            StatusCode::UNAUTHORIZED,
            "Abra o link com a chave do xau-web (no terminal, ou nos logs do servidor).",
        );
    }
    match assets::get("index.html") {
        Some(a) => secure(([(header::CONTENT_TYPE, a.mime)], a.bytes.into_owned()).into_response()),
        None => text(StatusCode::NOT_FOUND, "sem interface"),
    }
}

async fn asset(State(app): State<App>, h: HeaderMap, Path(path): Path<String>) -> Response {
    if !app.access.host_ok(&h) {
        return text(StatusCode::FORBIDDEN, "host não permitido");
    }
    match assets::get(&path) {
        Some(a) => secure(([(header::CONTENT_TYPE, a.mime)], a.bytes.into_owned()).into_response()),
        None => text(StatusCode::NOT_FOUND, "não encontrado"),
    }
}

async fn info(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(r) = guard(&app, &h, false) {
        return r;
    }
    json_response(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "nativeWindows": app.host.native(),
        "signIn": true,
        "lan": app.access.lan,
        "public": app.access.public(),
        "egress": app.egress.snapshot(),
        "dataDir": xau_mobile::accounts::data_dir().map(|p| p.display().to_string()).ok(),
    }))
}

// ------------------------------------------------------------- commands

/// What the UI may ask of the core. The sign-in plumbing and the checkout
/// form (which carries a Store token) stay on this side of the fence.
const ALLOWED: &[&str] = &[
    "restore", "refresh", "select", "sign_out", "set_frozen", "single_active", "set_single_active",
    "missions", "rewards", "level_log", "claim", "claim_all", "offers_all", "offer_codes",
    "offer_history", "offer_csv", "claim_offers_all", "store_search", "store_browse", "store_prewarm",
    "store_featured", "store_purchase_url", "farm_summary", "farm_titles", "farm_next_pool", "farm_pool",
    "farm_start", "farm_stop", "farm_state", "public_ip",
];

async fn call(State(app): State<App>, h: HeaderMap, Path(method): Path<String>, body: Bytes) -> Response {
    if let Err(r) = guard(&app, &h, true) {
        return r;
    }
    if !ALLOWED.contains(&method.as_str()) {
        return err(format!("método desconhecido: {method}"));
    }
    // Nothing goes to Microsoft except through Cloudflare, once that is confirmed.
    if let Err(e) = app.egress.require() {
        return err(e);
    }
    let args: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let xuid = args.get("xuid").and_then(Value::as_str).map(str::to_string);

    let m = method.clone();
    let done = tokio::spawn(async move { xau_api::dispatch(&m, &args).await }).await;
    match done {
        Ok(Ok(v)) => {
            // Everything its pages stored goes with the account.
            if method == "sign_out" {
                if let Some(x) = xuid {
                    profiles::forget(&x);
                }
            }
            ok(v)
        }
        Ok(Err(e)) => err(e),
        Err(_) => err("falha interna inesperada"),
    }
}

// -------------------------------------------------------------- sign-in

async fn login_start(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(r) = guard(&app, &h, true) {
        return r;
    }
    if let Err(e) = app.egress.require() {
        return err(e);
    }
    match signin::start(app.host.clone(), app.logins.clone()).await {
        Ok(id) => ok(json!({ "id": id })),
        Err(e) => err(e),
    }
}

async fn login_status(State(app): State<App>, h: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&app, &h, false) {
        return r;
    }
    match app.logins.get(&id) {
        Some(s) => ok(s.to_json()),
        None => err("login desconhecido"),
    }
}

async fn login_cancel(State(app): State<App>, h: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&app, &h, true) {
        return r;
    }
    app.logins.cancel(&app.host, &id).await;
    ok(json!({}))
}

/// The sign-in page as a picture, for the site to draw (see `signin`).
async fn login_frame(State(app): State<App>, h: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&app, &h, false) {
        return r;
    }
    let n = |k: &str, d: u32| q.get(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(d);
    match app.logins.frame(&app.host, &id, n("w", 412), n("h", 860), n("d", 175)).await {
        Ok(f) => {
            let mut r = secure(([(header::CONTENT_TYPE, "image/jpeg")], f.jpeg).into_response());
            r.headers_mut().insert("x-edit", HeaderValue::from_static(if f.editing { "1" } else { "0" }));
            r
        }
        Err(e) => {
            tracing::debug!("login frame: {e}");
            text(StatusCode::GONE, "sem imagem")
        }
    }
}

/// The sign-in page, live: every frame the engine draws, as an MJPEG stream an `<img>` plays
/// by itself. It ends when the sign-in does.
async fn login_stream(State(app): State<App>, h: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    use futures_util::stream;
    if let Err(r) = guard(&app, &h, false) {
        return r;
    }
    let n = |k: &str, d: u32| q.get(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(d);
    let rx = match app.logins.stream(&app.host, &id, n("w", 412), n("h", 860), n("d", 175)).await {
        Ok(rx) => rx,
        Err(e) => {
            tracing::debug!("login stream: {e}");
            return text(StatusCode::GONE, "sem imagem ao vivo");
        }
    };
    fn part(jpeg: &Bytes) -> Bytes {
        let head = format!("--xauframe\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", jpeg.len());
        let mut b = Vec::with_capacity(head.len() + jpeg.len() + 2);
        b.extend_from_slice(head.as_bytes());
        b.extend_from_slice(jpeg);
        b.extend_from_slice(b"\r\n");
        Bytes::from(b)
    }
    // The current picture first, then one part per new frame.
    let body = stream::unfold((rx, true), |(mut rx, first)| async move {
        loop {
            if !first || rx.borrow().is_none() {
                rx.changed().await.ok()?;
            }
            let frame = rx.borrow_and_update().clone();
            if let Some(jpeg) = frame {
                return Some((Ok::<_, std::convert::Infallible>(part(&jpeg)), (rx, false)));
            }
        }
    });
    let mut r = secure(axum::body::Body::from_stream(body).into_response());
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("multipart/x-mixed-replace; boundary=xauframe"));
    r.headers_mut().insert("x-accel-buffering", HeaderValue::from_static("no"));
    r
}

/// The sign-in page over one WebSocket: the newest frame as a binary message each time the
/// page draws, and the person's input as text messages, replayed in order. After an input
/// that may move the focus, a text message says whether a field has it.
async fn login_ws(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    if let Err(r) = guard(&app, &h, false) {
        return r;
    }
    if !app.access.ws_origin_ok(&h) {
        return text(StatusCode::FORBIDDEN, "origem não permitida");
    }
    let n = |k: &str, d: u32| q.get(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(d);
    let frames = match app.logins.stream(&app.host, &id, n("w", 412), n("h", 860), n("d", 175)).await {
        Ok(rx) => rx,
        Err(e) => {
            tracing::debug!("login ws: {e}");
            return text(StatusCode::GONE, "sem imagem ao vivo");
        }
    };
    ws.max_message_size(64 * 1024).on_upgrade(move |socket| login_ws_session(app, id, frames, socket))
}

async fn login_ws_session(
    app: App,
    id: String,
    mut frames: tokio::sync::watch::Receiver<Option<Bytes>>,
    mut socket: axum::extract::ws::WebSocket,
) {
    use axum::extract::ws::Message;
    let (reply_tx, mut replies) = tokio::sync::mpsc::unbounded_channel::<bool>();
    // Once the page holds still, a sharp picture replaces the last (CSS-pixel) screencast frame.
    const STILL: std::time::Duration = std::time::Duration::from_millis(120);
    let mut sharpen_at: Option<tokio::time::Instant> = None;
    // Taking the sharp picture makes the engine draw again, the same picture: only a frame that
    // differs from the last one is news (else the two would take turns forever).
    let mut last_fast: Option<Bytes> = None;
    // ...and the frames right after a sharp picture are its echo, until the person does something.
    let mut quiet_until: Option<tokio::time::Instant> = None;
    // While the person is not touching anything, a small change (a blinking caret) goes out as a
    // sharp picture only, not blurry-then-sharp; changes that keep coming (an animation) go out fast.
    let mut last_input = tokio::time::Instant::now() - std::time::Duration::from_secs(10);
    let mut pending_since: Option<tokio::time::Instant> = None;
    // The picture there already, if any, goes first.
    let first = frames.borrow_and_update().clone();
    if let Some(jpeg) = first {
        if socket.send(Message::Binary(jpeg.to_vec())).await.is_err() {
            return;
        }
    }
    loop {
        tokio::select! {
            changed = frames.changed() => {
                if changed.is_err() {
                    break; // the sign-in window closed
                }
                let jpeg = frames.borrow_and_update().clone();
                if jpeg.is_some() && jpeg == last_fast {
                    continue;
                }
                if quiet_until.is_some_and(|t| tokio::time::Instant::now() < t) {
                    continue;
                }
                last_fast.clone_from(&jpeg);
                if let Some(jpeg) = jpeg {
                    let now = tokio::time::Instant::now();
                    let active = now - last_input < std::time::Duration::from_secs(1);
                    let moving = pending_since.is_some_and(|t| now - t > std::time::Duration::from_millis(200));
                    if active || moving {
                        if socket.send(Message::Binary(jpeg.to_vec())).await.is_err() {
                            break;
                        }
                    } else {
                        pending_since.get_or_insert(now);
                    }
                    sharpen_at = Some(now + STILL);
                }
            }
            () = async { tokio::time::sleep_until(sharpen_at.unwrap_or_else(tokio::time::Instant::now)).await }, if sharpen_at.is_some() => {
                sharpen_at = None;
                if let Ok(jpeg) = app.logins.sharp_frame(&app.host, &id).await {
                    if socket.send(Message::Binary(jpeg)).await.is_err() {
                        break;
                    }
                    quiet_until = Some(tokio::time::Instant::now() + std::time::Duration::from_millis(150));
                    pending_since = None;
                }
            }
            msg = socket.recv() => {
                let Some(Ok(msg)) = msg else { break };
                let Message::Text(body) = msg else { continue };
                let Ok(ev) = serde_json::from_str::<Value>(&body) else { continue };
                quiet_until = None; // the page answers the person at once
                last_input = tokio::time::Instant::now();
                match app.logins.dispatch_input(&app.host, &id, &ev).await {
                    Ok(true) => {
                        let (logins, host, id, tx) = (app.logins.clone(), app.host.clone(), id.clone(), reply_tx.clone());
                        tokio::spawn(async move {
                            let _ = tx.send(logins.editing_after_input(&host, &id).await);
                        });
                    }
                    Ok(false) => {}
                    Err(e) => tracing::debug!("login ws input: {e}"),
                }
            }
            () = app.logins.swapped.notified() => break, // another window is shown: the page reconnects
            Some(editing) = replies.recv() => {
                if socket.send(Message::Text(json!({ "editing": editing }).to_string())).await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn login_input(State(app): State<App>, h: HeaderMap, Path(id): Path<String>, body: Bytes) -> Response {
    if let Err(r) = guard(&app, &h, true) {
        return r;
    }
    let ev: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return err("corpo inválido"),
    };
    match app.logins.input(&app.host, &id, &ev).await {
        Ok(editing) => ok(json!({ "editing": editing })),
        Err(e) => err(e),
    }
}

// ------------------------------------------------------------ benchmark

fn bench_on() -> bool {
    std::env::var_os("XAU_WEB_BENCH").is_some()
}

/// Benchmark mode only: open the local test page in the sign-in's hidden window.
async fn bench_start(State(app): State<App>, h: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if !bench_on() {
        return text(StatusCode::NOT_FOUND, "não encontrado");
    }
    if let Err(r) = guard(&app, &h, true) {
        return r;
    }
    let anim = q.get("anim").is_some_and(|v| v == "1");
    let url = format!("http://127.0.0.1:{}/bench-page{}", app.port, if anim { "#anim" } else { "" });
    // A real page for comparison: only Microsoft's sign-up (no account, nothing registered).
    let url = match q.get("url") {
        Some(u) if u.starts_with("https://signup.live.com/") || u == "https://www.cloudflare.com/cdn-cgi/trace" => u.clone(),
        _ => url,
    };
    let windows_ua = q.get("ua").is_some_and(|v| v == "win");
    match signin::start_bench(app.host.clone(), app.logins.clone(), url, windows_ua).await {
        Ok(id) => ok(json!({ "id": id })),
        Err(e) => err(e),
    }
}

/// Benchmark mode only: a page that repaints on every touch (static mode) or on every frame
/// (`#anim`), so frame rate and touch-to-frame latency can be measured without Microsoft.
async fn bench_page() -> Response {
    if !bench_on() {
        return text(StatusCode::NOT_FOUND, "não encontrado");
    }
    const PAGE: &str = r#"<!doctype html><meta name=viewport content="width=device-width,initial-scale=1">
<style>html,body{margin:0;height:100%;font:20px sans-serif;background:#fff}#b{position:absolute;top:120px;left:0;width:60px;height:60px;background:#107c10}#n{position:absolute;top:20px;left:20px}</style>
<div id=n>0</div><div id=b></div><input id=t style="position:absolute;top:300px;left:20px;width:300px;font-size:20px">
<script>var n=0,anim=location.hash==='#anim',on=false;
document.addEventListener('touchstart',function(e){if(e.target.id==='t')return;on=!on;document.body.style.background=on?'#222':'#fff';n++;document.getElementById('n').textContent=n;},{passive:true});
if(anim){(function f(t){document.getElementById('b').style.transform='translateX('+(Math.sin(t/300)*150+170)+'px)';requestAnimationFrame(f)})(0)}</script>"#;
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], PAGE).into_response()
}

// ------------------------------------------------------------- checkout

/// A JavaScript string literal for `s`. JSON is almost one: only the two
/// line separators differ.
fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()).replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}

/// Builds and submits the checkout form inside the page, so it is posted
/// with that page's origin and the token stays in the request body.
fn submit_script(action: &str, fields: &[(String, String)]) -> String {
    let mut s = format!("var f=document.createElement('form');f.method='POST';f.action={};", js_str(action));
    for (name, value) in fields {
        s.push_str(&format!(
            "var i=document.createElement('input');i.type='hidden';i.name={};i.value={};f.appendChild(i);",
            js_str(name),
            js_str(value)
        ));
    }
    s.push_str("document.documentElement.appendChild(f);f.submit();");
    s
}

async fn checkout(State(app): State<App>, h: HeaderMap, body: Bytes) -> Response {
    if let Err(r) = guard(&app, &h, true) {
        return r;
    }
    let args: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let (Some(xuid), Some(product)) = (args["xuid"].as_str(), args["productId"].as_str()) else {
        return err("faltou a conta ou o jogo");
    };

    if let Err(e) = app.egress.require() {
        return err(e);
    }
    let _one_at_a_time = app.checkout.lock().await;
    if let Err(e) = xau_api::dispatch("select", &json!({ "xuid": xuid })).await {
        return err(e);
    }
    // Signed in the way the Xbox app does it; an account without the
    // Windows sign-in gets the public product page instead.
    let spec = match xau_api::dispatch("store_purchase_form", &json!({ "productId": product })).await {
        Ok(form) => {
            let fields: Vec<(String, String)> = form["fields"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            CheckoutSpec {
                title: "Comprar na Microsoft Store".into(),
                host_url: form["host"].as_str().unwrap_or_default().to_string(),
                submit_script: submit_script(form["action"].as_str().unwrap_or_default(), &fields),
                profile_dir: profiles::of(xuid),
            }
        }
        Err(e) => {
            tracing::warn!("checkout without sign-in: {e}");
            let url = xau_api::dispatch("store_purchase_url", &json!({ "productId": product }))
                .await
                .ok()
                .and_then(|v| v["url"].as_str().map(str::to_string))
                .unwrap_or_default();
            CheckoutSpec {
                title: "Microsoft Store".into(),
                host_url: url,
                submit_script: String::new(),
                profile_dir: profiles::of(xuid),
            }
        }
    };
    match app.host.open_checkout(spec) {
        Ok(()) => ok(json!({})),
        Err(e) => err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checkout_form_is_built_without_breaking_out_of_its_strings() {
        let s = submit_script(
            "https://www.microsoft.com/x?a=1",
            &[("n\"ame".into(), "v</script>\u{2028}".into())],
        );
        assert!(s.contains(r#"f.action="https://www.microsoft.com/x?a=1""#));
        assert!(s.contains(r#"i.name="n\"ame""#));
        assert!(s.contains(r"\u2028"), "a line separator is escaped");
        assert!(!s.contains('\u{2028}'));
    }

    #[test]
    fn the_sign_in_plumbing_and_the_checkout_token_are_not_exposed() {
        for m in ["windows_login_start", "windows_login_message", "windows_login_cancel", "login_url", "on_navigate", "finish_login", "store_purchase_form"] {
            assert!(!ALLOWED.contains(&m), "{m} must stay server-side");
        }
        assert!(ALLOWED.contains(&"restore") && ALLOWED.contains(&"farm_start"));
    }
}
