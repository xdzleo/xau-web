//! Native browser windows, for the two flows a web page cannot do itself.
//!
//! - **Sign-in.** Microsoft's Windows sign-in page must be loaded with a
//!   special user agent and headers, with a script injected at the start
//!   of every document, and its answer read from a bridge. None of that is
//!   possible from an `<iframe>` or a popup of a normal web page, and
//!   proxying Microsoft's login through this server would put the user's
//!   password in our hands. So the server opens a real window (WebView2,
//!   the same engine the Android app uses its WebView for) and the user
//!   types the password into Microsoft's own page, there.
//! - **Checkout.** The Store's purchase page only accepts the form from
//!   a page on its own origin, so it is posted from inside such a window,
//!   in the account's own profile.
//!
//! The windows need an event loop on the main thread, so `main` runs it
//! and the HTTP server talks to it through a [`Host`] handle.

use std::path::PathBuf;

use tokio::sync::mpsc::UnboundedReceiver;

/// What a sign-in window tells the server.
#[derive(Debug)]
pub enum WinEvent {
    /// The page posted a message to the host (a token, a context request).
    Message(String),
    /// A page finished loading.
    PageFinished(String),
    /// The sign-in page is leaving for Microsoft's sign-up (`to_signup`), or the sign-up window
    /// for the sign-in: the sign-in decides where `url` opens.
    Leave { url: String, to_signup: bool },
    /// The user closed the window.
    Closed,
    /// The window could not be opened.
    Failed(String),
}

pub struct SignInSpec {
    pub title: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub user_agent: String,
    /// Run at the start of every document, every frame.
    pub init_script: String,
    /// The name the page's script calls: `XauLogin.postMessage(...)`.
    pub bridge: String,
    pub profile_dir: PathBuf,
    /// Draw the page off screen: the web page shows it (see `signin::Logins::frame`).
    pub hidden: bool,
    /// Where the page's screencast frames go (JPEG), when it is drawn inside the site.
    pub frames: Option<tokio::sync::watch::Sender<Option<bytes::Bytes>>>,
    /// The window of Microsoft's sign-up, opened beside the sign-in (see `signin`): it hands
    /// back any way to the sign-in instead of following it.
    pub side: bool,
}

pub struct CheckoutSpec {
    pub title: String,
    /// The page to load first, on the checkout's own origin.
    pub host_url: String,
    /// Posts the form; runs once, on `host_url`. Empty: just show the page.
    pub submit_script: String,
    pub profile_dir: PathBuf,
}

pub type SignInHandle = (u64, UnboundedReceiver<WinEvent>);

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
    use tao::window::{Window, WindowBuilder, WindowId};
    use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
    use tokio::sync::oneshot;
    use wry::http::{HeaderMap, HeaderName, HeaderValue};
    use wry::{PageLoadEvent, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows};

    use super::{unhosted_signup, CheckoutSpec, SignInHandle, SignInSpec, WinEvent};

    pub enum Cmd {
        SignIn { id: u64, spec: SignInSpec, tx: UnboundedSender<WinEvent> },
        Checkout { id: u64, spec: CheckoutSpec },
        Eval(u64, String),
        Navigate(u64, String),
        /// Load a page with these headers (used once the page's device emulation is set).
        Load(u64, String, Vec<(String, String)>),
        Close(u64),
        /// A DevTools protocol call on the window's page; the answer is the JSON it returns.
        Cdp { id: u64, method: String, params: String, reply: oneshot::Sender<Result<String, String>> },
    }

    #[derive(Clone)]
    pub struct Host {
        proxy: EventLoopProxy<Cmd>,
        next: Arc<AtomicU64>,
    }

    pub struct Loop(EventLoop<Cmd>);

    pub fn create() -> (Host, Loop) {
        let event_loop = EventLoopBuilder::<Cmd>::with_user_event().build();
        let host = Host { proxy: event_loop.create_proxy(), next: Arc::new(AtomicU64::new(1)) };
        (host, Loop(event_loop))
    }

    impl Host {
        fn send(&self, cmd: Cmd) -> Result<(), String> {
            self.proxy.send_event(cmd).map_err(|_| "o laço de janelas parou".to_string())
        }

        pub fn open_sign_in(&self, spec: SignInSpec) -> Result<SignInHandle, String> {
            let id = self.next.fetch_add(1, Ordering::SeqCst);
            let (tx, rx) = unbounded_channel();
            self.send(Cmd::SignIn { id, spec, tx })?;
            Ok((id, rx))
        }

        pub fn open_checkout(&self, spec: CheckoutSpec) -> Result<(), String> {
            let id = self.next.fetch_add(1, Ordering::SeqCst);
            self.send(Cmd::Checkout { id, spec })
        }

        pub fn eval(&self, win: u64, js: String) {
            let _ = self.send(Cmd::Eval(win, js));
        }

        pub fn navigate(&self, win: u64, url: String) {
            let _ = self.send(Cmd::Navigate(win, url));
        }

        pub fn load(&self, win: u64, url: String, headers: Vec<(String, String)>) {
            let _ = self.send(Cmd::Load(win, url, headers));
        }

        pub fn close(&self, win: u64) {
            let _ = self.send(Cmd::Close(win));
        }

        pub async fn cdp(&self, win: u64, method: &str, params: &str) -> Result<String, String> {
            let (reply, rx) = oneshot::channel();
            self.send(Cmd::Cdp { id: win, method: method.into(), params: params.into(), reply })?;
            match tokio::time::timeout(std::time::Duration::from_secs(8), rx).await {
                Ok(Ok(r)) => r,
                Ok(Err(_)) => Err("a janela fechou".into()),
                Err(_) => Err("a página não respondeu".into()),
            }
        }
    }

    /// One open window. Field order is drop order: the page goes before
    /// its profile and the window it is drawn in.
    struct Entry {
        handle: u64,
        webview: WebView,
        _ctx: Box<WebContext>,
        _window: Window,
        tx: Option<UnboundedSender<WinEvent>>,
    }

    fn headers_of(pairs: &[(String, String)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            if let (Ok(n), Ok(val)) = (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v)) {
                map.insert(n, val);
            }
        }
        map
    }

    fn window(target: &EventLoopWindowTarget<Cmd>, title: &str, w: f64, h: f64, hidden: bool) -> Result<Window, String> {
        use tao::dpi::PhysicalPosition;
        use tao::platform::windows::WindowBuilderExtWindows;
        let mut b = WindowBuilder::new().with_title(title).with_inner_size(LogicalSize::new(w, h));
        if hidden {
            // Still "shown", so the page keeps drawing, but nowhere a person looks.
            b = b.with_position(PhysicalPosition::new(-20000, -20000)).with_skip_taskbar(true).with_resizable(false);
        }
        let window = b.build(target).map_err(|e| format!("não foi possível abrir a janela: {e}"))?;
        if !hidden {
            window.set_focus();
        }
        Ok(window)
    }

    /// Add `headers` to the sign-in page's own request, and to nothing else: the app's WebView
    /// sends them on the first request only, and the page's calls to other hosts break with them.
    /// (WebView2 drops the headers of a programmatic load, and without them Microsoft serves
    /// the Windows page instead of the one the app shows.)
    fn add_first_request_headers(webview: &WebView, headers: &[(String, String)]) -> Result<(), String> {
        use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT;
        use webview2_com::WebResourceRequestedEventHandler;
        use windows::core::HSTRING;
        use windows::Win32::System::WinRT::EventRegistrationToken;
        use wry::WebViewExtWindows;

        let e = |e: windows::core::Error| e.to_string();
        let core = unsafe { webview.controller().CoreWebView2() }.map_err(e)?;
        let headers = headers.to_vec();
        let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                unsafe {
                    if let Ok(h) = args.Request().and_then(|r| r.Headers()) {
                        for (k, v) in &headers {
                            let _ = h.SetHeader(&HSTRING::from(k.as_str()), &HSTRING::from(v.as_str()));
                        }
                    }
                }
            }
            Ok(())
        }));
        let mut token = EventRegistrationToken::default();
        unsafe {
            core.AddWebResourceRequestedFilter(
                &HSTRING::from("https://login.live.com/ppsecure/InlineLogin.srf*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
            )
            .map_err(e)?;
            core.add_WebResourceRequested(&handler, &mut token).map_err(e)?;
        }
        Ok(())
    }

    /// Publish every screencast frame the engine pushes (`Page.startScreencast`) as soon as it
    /// comes, acknowledged at once so the next one is not held back.
    fn on_screencast_frames(webview: &WebView, frames: tokio::sync::watch::Sender<Option<bytes::Bytes>>) -> Result<(), String> {
        use base64::Engine;
        use webview2_com::{take_pwstr, CallDevToolsProtocolMethodCompletedHandler, DevToolsProtocolEventReceivedEventHandler};
        use windows::core::{HSTRING, PWSTR};
        use windows::Win32::System::WinRT::EventRegistrationToken;
        use wry::WebViewExtWindows;

        let e = |e: windows::core::Error| e.to_string();
        let core = unsafe { webview.controller().CoreWebView2() }.map_err(e)?;
        let receiver = unsafe { core.GetDevToolsProtocolEventReceiver(&HSTRING::from("Page.screencastFrame")) }.map_err(e)?;
        let acker = core.clone();
        let handler = DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut json = PWSTR::null();
            unsafe { args.ParameterObjectAsJson(&mut json)? };
            let json = take_pwstr(json);
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) else { return Ok(()) };
            if let Some(session) = v["sessionId"].as_i64() {
                let params = HSTRING::from(format!("{{\"sessionId\":{session}}}"));
                let done = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
                let _ = unsafe { acker.CallDevToolsProtocolMethod(&HSTRING::from("Page.screencastFrameAck"), &params, &done) };
            }
            if let Ok(jpeg) = base64::engine::general_purpose::STANDARD.decode(v["data"].as_str().unwrap_or("")) {
                frames.send_replace(Some(bytes::Bytes::from(jpeg)));
            }
            Ok(())
        }));
        let mut token = EventRegistrationToken::default();
        unsafe { receiver.add_DevToolsProtocolEventReceived(&handler, &mut token) }.map_err(e)?;
        Ok(())
    }

    /// Run one DevTools protocol call on the page, answering through `reply`.
    fn cdp_call(webview: &WebView, method: &str, params: &str, reply: oneshot::Sender<Result<String, String>>) {
        use std::sync::Mutex;
        use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
        use windows::core::HSTRING;
        use wry::WebViewExtWindows;

        let slot = Arc::new(Mutex::new(Some(reply)));
        let done = slot.clone();
        let fail = move |e: String| {
            if let Some(tx) = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
                let _ = tx.send(Err(e));
            }
        };
        let core = match unsafe { webview.controller().CoreWebView2() } {
            Ok(c) => c,
            Err(e) => return fail(e.to_string()),
        };
        let (m, p) = (HSTRING::from(method), HSTRING::from(params));
        let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |res, json: String| {
            if let Some(tx) = done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
                let _ = tx.send(res.map(|()| json).map_err(|e| e.to_string()));
            }
            Ok(())
        }));
        if let Err(e) = unsafe { core.CallDevToolsProtocolMethod(&m, &p, &handler) } {
            fail(e.to_string());
        }
    }

    fn sign_in(
        target: &EventLoopWindowTarget<Cmd>,
        proxy: EventLoopProxy<Cmd>,
        id: u64,
        spec: SignInSpec,
        tx: UnboundedSender<WinEvent>,
    ) -> Result<Entry, String> {
        let window = window(target, &spec.title, 412.0, 860.0, spec.hidden)?;
        let mut ctx = Box::new(WebContext::new(Some(spec.profile_dir.clone())));

        // The page calls `window.external.notify(m)`, which the core's init
        // script turns into `<bridge>.postMessage(m)`; here that lands in
        // WebView2's own channel.
        // The channel is captured first, then WebView2's own objects are hidden: the app's
        // WebView has neither `window.chrome.webview` nor `window.ipc`, and Microsoft's
        // page changes with what it sees there.
        let init = format!(
            "(function () {{ var cw = window.chrome && window.chrome.webview; \
             window.{b} = {{ postMessage: function (m) {{ (cw ? cw : window.ipc).postMessage(String(m)); }} }}; \
             try {{ delete window.chrome.webview; }} catch (e) {{}} \
             try {{ Object.defineProperty(window.chrome, 'webview', {{ get: function () {{ return undefined; }}, configurable: true }}); }} catch (e) {{}} \
             try {{ delete window.ipc; }} catch (e) {{}} }})();\n\
             // Not on the sign-up: it would take itself for the Windows-hosted one and stop.\n\
             if (location.hostname !== 'signup.live.com') {{ {s} }}",
            b = spec.bridge,
            s = spec.init_script
        );
        let (tx_ipc, tx_load) = (tx.clone(), tx.clone());
        let proxy_nav = proxy.clone();
        // A page that draws inside the site is loaded only once its device emulation is set (see `signin`).
        let (first_url, first_headers) = if spec.hidden {
            ("about:blank".to_string(), HeaderMap::new())
        } else {
            (spec.url.clone(), headers_of(&spec.headers))
        };
        // wry's own flags, plus: a window drawn off screen is "occluded" to the engine, which then
        // slows its frames down; the page shown inside the site must run at full rate.
        let mut browser_args = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
        if spec.hidden {
            browser_args = String::from(
                "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion \
                 --disable-backgrounding-occluded-windows --disable-renderer-backgrounding --disable-background-timer-throttling",
            );
        }
        let webview = WebViewBuilder::with_web_context(&mut ctx)
            .with_additional_browser_args(browser_args)
            .with_user_agent(spec.user_agent.clone())
            // Every frame, like the Windows host: the pages Microsoft asks
            // for next are not all on one origin.
            .with_initialization_script_for_main_only(&init, false)
            // A link that asks for a new window (target=_blank, window.open) opens in this same page,
            // as in the app's WebView, instead of in a window nobody can see.
            .with_new_window_req_handler(move |url| {
                let _ = proxy_nav.send_event(Cmd::Navigate(id, url));
                false
            })
            // "Create one!" opens Microsoft's sign-up in the Windows-hosted mode, which waits forever
            // for answers only Windows' own host gives. The sign-in opens it as the plain web sign-up
            // in a clean window of its own instead; that window hands any way back to the sign-in.
            .with_navigation_handler({
                let tx_nav = tx.clone();
                let side = spec.side;
                move |url| {
                    tracing::debug!("sign-in navigates: {url}");
                    if !side {
                        if let Some(plain) = unhosted_signup(&url) {
                            let _ = tx_nav.send(WinEvent::Leave { url: plain, to_signup: true });
                            return false;
                        }
                    // Only the sign-up's way back (its `bu`/`sru`): it passes through login.live.com otherwise.
                    } else if url.starts_with("https://login.live.com/ppsecure/InlineLogin.srf") {
                        let _ = tx_nav.send(WinEvent::Leave { url, to_signup: false });
                        return false;
                    }
                    true
                }
            })
            .with_ipc_handler(move |req| {
                let _ = tx_ipc.send(WinEvent::Message(req.into_body()));
            })
            .with_on_page_load_handler(move |ev, url| {
                if matches!(ev, PageLoadEvent::Finished) {
                    let _ = tx_load.send(WinEvent::PageFinished(url));
                }
            })
            .with_url_and_headers(first_url, first_headers)
            .build(&window)
            .map_err(|e| format!("não foi possível abrir o navegador (WebView2): {e}"))?;
        if spec.hidden {
            add_first_request_headers(&webview, &spec.headers)?;
        }
        if let Some(frames) = spec.frames {
            on_screencast_frames(&webview, frames)?;
        }
        Ok(Entry { handle: id, webview, _ctx: ctx, _window: window, tx: Some(tx) })
    }

    fn checkout(target: &EventLoopWindowTarget<Cmd>, id: u64, spec: CheckoutSpec) -> Result<Entry, String> {
        let window = window(target, &spec.title, 560.0, 840.0, false)?;
        let mut ctx = Box::new(WebContext::new(Some(spec.profile_dir.clone())));

        // Posted from inside the page, once, when it is the host page: the
        // checkout refuses a form that does not come from its own origin.
        let host = serde_json::to_string(spec.host_url.split('?').next().unwrap_or("")).unwrap_or_default();
        let init = if spec.submit_script.is_empty() {
            String::new()
        } else {
            format!(
                "(function () {{ if (location.href.split('?')[0] !== {host}) return; \
                 try {{ if (sessionStorage.getItem('xau-posted')) return; sessionStorage.setItem('xau-posted', '1'); }} catch (e) {{}} \
                 function go() {{ {script} }} \
                 if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', go); else go(); }})();",
                script = spec.submit_script
            )
        };
        let webview = WebViewBuilder::with_web_context(&mut ctx)
            .with_initialization_script_for_main_only(&init, true)
            .with_url(spec.host_url.clone())
            .build(&window)
            .map_err(|e| format!("não foi possível abrir o navegador (WebView2): {e}"))?;
        Ok(Entry { handle: id, webview, _ctx: ctx, _window: window, tx: None })
    }

    impl Loop {
        /// Run the windows until the process ends. Call on the main thread.
        pub fn run(self) -> ! {
            let mut entries: HashMap<WindowId, Entry> = HashMap::new();
            let mut by_handle: HashMap<u64, WindowId> = HashMap::new();
            let proxy = self.0.create_proxy();

            self.0.run(move |event, target, flow| {
                *flow = ControlFlow::Wait;
                match event {
                    Event::UserEvent(Cmd::SignIn { id, spec, tx }) => match sign_in(target, proxy.clone(), id, spec, tx.clone()) {
                        Ok(entry) => {
                            let wid = entry._window.id();
                            by_handle.insert(id, wid);
                            entries.insert(wid, entry);
                        }
                        Err(e) => {
                            let _ = tx.send(WinEvent::Failed(e));
                        }
                    },
                    Event::UserEvent(Cmd::Checkout { id, spec }) => match checkout(target, id, spec) {
                        Ok(entry) => {
                            let wid = entry._window.id();
                            by_handle.insert(id, wid);
                            entries.insert(wid, entry);
                        }
                        Err(e) => tracing::warn!("checkout window: {e}"),
                    },
                    Event::UserEvent(Cmd::Eval(id, js)) => {
                        if let Some(e) = by_handle.get(&id).and_then(|w| entries.get(w)) {
                            let _ = e.webview.evaluate_script(&js);
                        }
                    }
                    Event::UserEvent(Cmd::Navigate(id, url)) => {
                        if let Some(e) = by_handle.get(&id).and_then(|w| entries.get(w)) {
                            let _ = e.webview.load_url(&url);
                        }
                    }
                    Event::UserEvent(Cmd::Cdp { id, method, params, reply }) => match by_handle.get(&id).and_then(|w| entries.get(w)) {
                        Some(e) => cdp_call(&e.webview, &method, &params, reply),
                        None => {
                            let _ = reply.send(Err("a janela fechou".into()));
                        }
                    },
                    Event::UserEvent(Cmd::Load(id, url, headers)) => {
                        if let Some(e) = by_handle.get(&id).and_then(|w| entries.get(w)) {
                            let _ = e.webview.load_url_with_headers(&url, headers_of(&headers));
                        }
                    }
                    Event::UserEvent(Cmd::Close(id)) => {
                        // Closed by us: nobody is told, they asked for it.
                        if let Some(w) = by_handle.remove(&id) {
                            entries.remove(&w);
                        }
                    }
                    Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. } => {
                        if let Some(entry) = entries.remove(&window_id) {
                            by_handle.remove(&entry.handle);
                            if let Some(tx) = &entry.tx {
                                let _ = tx.send(WinEvent::Closed);
                            }
                        }
                    }
                    _ => {}
                }
            })
        }
    }
}

/// The sign-up URL without the Windows-hosted mode (`hosted`, `platform`, `uiflavor=win10host`,
/// `cxh-*`), when `url` is Microsoft's sign-up in that mode.
pub fn unhosted_signup(url: &str) -> Option<String> {
    let (base, query) = url.split_once('?')?;
    if !base.starts_with("https://signup.live.com/") {
        return None;
    }
    let hosted = |p: &&str| {
        let k = p.split('=').next().unwrap_or("").to_ascii_lowercase();
        k == "hosted" || k == "platform" || k == "uiflavor" || k.starts_with("cxh-")
    };
    if !query.split('&').any(|p| hosted(&p)) {
        return None;
    }
    let kept: Vec<&str> = query.split('&').filter(|p| !hosted(p)).collect();
    Some(if kept.is_empty() { base.to_string() } else { format!("{base}?{}", kept.join("&")) })
}

/// The browser behind the windows. On Windows, the system's WebView2 in real windows (the default);
/// everywhere (and by `XAU_BACKEND=chromium` on Windows), headless Chromium over DevTools, which
/// needs no screen: that is what a server runs.
#[derive(Clone)]
pub enum Host {
    #[cfg(windows)]
    Native(imp::Host),
    Cdp(crate::cdphost::Host),
}

/// What `main` runs on its thread: the native windows' event loop, or nothing.
pub enum Loop {
    #[cfg(windows)]
    Native(imp::Loop),
    Idle,
}

pub fn create() -> (Host, Loop) {
    #[cfg(windows)]
    if std::env::var("XAU_BACKEND").map_or(true, |b| b != "chromium") {
        let (host, event_loop) = imp::create();
        return (Host::Native(host), Loop::Native(event_loop));
    }
    (Host::Cdp(crate::cdphost::Host::new()), Loop::Idle)
}

impl Loop {
    pub fn run(self) -> ! {
        match self {
            #[cfg(windows)]
            Loop::Native(l) => l.run(),
            Loop::Idle => loop {
                std::thread::park();
            },
        }
    }
}

impl Host {
    /// Whether this server opens real windows on its own computer (the Store's purchase page too).
    pub fn native(&self) -> bool {
        match self {
            #[cfg(windows)]
            Host::Native(_) => true,
            Host::Cdp(_) => false,
        }
    }

    pub fn open_sign_in(&self, spec: SignInSpec) -> Result<SignInHandle, String> {
        match self {
            #[cfg(windows)]
            Host::Native(h) => h.open_sign_in(spec),
            Host::Cdp(h) => h.open_sign_in(spec),
        }
    }

    pub fn open_checkout(&self, spec: CheckoutSpec) -> Result<(), String> {
        match self {
            #[cfg(windows)]
            Host::Native(h) => h.open_checkout(spec),
            Host::Cdp(_) => {
                let _ = spec;
                Err("este servidor não tem janelas: a compra abre a página pública do jogo".into())
            }
        }
    }

    pub fn eval(&self, win: u64, js: String) {
        match self {
            #[cfg(windows)]
            Host::Native(h) => h.eval(win, js),
            Host::Cdp(h) => h.eval(win, js),
        }
    }

    pub fn navigate(&self, win: u64, url: String) {
        match self {
            #[cfg(windows)]
            Host::Native(h) => h.navigate(win, url),
            Host::Cdp(h) => h.navigate(win, url),
        }
    }

    pub fn load(&self, win: u64, url: String, headers: Vec<(String, String)>) {
        match self {
            #[cfg(windows)]
            Host::Native(h) => h.load(win, url, headers),
            Host::Cdp(h) => {
                let _ = headers;
                h.load(win, url);
            }
        }
    }

    pub fn close(&self, win: u64) {
        match self {
            #[cfg(windows)]
            Host::Native(h) => h.close(win),
            Host::Cdp(h) => h.close(win),
        }
    }

    pub async fn cdp(&self, win: u64, method: &str, params: &str) -> Result<String, String> {
        match self {
            #[cfg(windows)]
            Host::Native(h) => h.cdp(win, method, params).await,
            Host::Cdp(h) => h.cdp(win, method, params).await,
        }
    }
}