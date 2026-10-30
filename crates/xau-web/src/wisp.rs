//! A Wisp server, for the sign-in page that runs in the person's own browser (Scramjet).
//!
//! Wisp is a small protocol that carries many TCP streams over one WebSocket. The browser does
//! the TLS itself (libcurl in WebAssembly), so this only moves bytes: it never sees what Microsoft
//! and the person say to each other. What it does decide is *where* the bytes may go, and that is
//! the whole security of this endpoint:
//!
//! * only Microsoft's own hosts (and the captcha vendors its sign-in uses), on ports 443 and 80;
//! * only TCP;
//! * only once the exit is confirmed to be Cloudflare's (see `egress`), and only through that
//!   proxy: a stream is a SOCKS5 connection to it, never a direct one from this host.
//!
//! Protocol (v1): every message is `[type u8][stream id u32 LE][payload]`. CONNECT carries
//! `[stream type u8][port u16 LE][host]`, DATA the bytes, CONTINUE `[buffer left u32 LE]` (flow
//! control for what the browser sends), CLOSE `[reason u8]`. The server speaks first, with a
//! CONTINUE on stream 0 that says how many packets the browser may send ahead.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::egress::Egress;

const T_CONNECT: u8 = 0x01;
const T_DATA: u8 = 0x02;
const T_CONTINUE: u8 = 0x03;
const T_CLOSE: u8 = 0x04;

const R_VOLUNTARY: u8 = 0x02;
const R_NETWORK: u8 = 0x03;
const R_INVALID: u8 = 0x41;
const R_UNREACHABLE: u8 = 0x42;
const R_NO_RESPONSE: u8 = 0x43;
const R_REFUSED: u8 = 0x44;
const R_BLOCKED: u8 = 0x48;
const R_THROTTLED: u8 = 0x49;

/// How many packets the browser may have in flight per stream.
const BUFFER: u32 = 128;
/// Streams open at once on one connection (a sign-in page opens a handful).
const MAX_STREAMS: usize = 48;
/// Connections open at once, all people together.
const MAX_SESSIONS: usize = 150;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const PING_EVERY: Duration = Duration::from_secs(25);
/// A stream with no bytes either way for this long is closed. The WARP tunnel drops connections left
/// idle without telling anyone, and the browser would send the page's next request (the email's
/// lookup, a link) down one and wait for an answer that never comes. Closed, it opens a new one.
const IDLE_CLOSE: Duration = Duration::from_secs(20);

static SESSIONS: AtomicUsize = AtomicUsize::new(0);

/// Where a stream may go: Microsoft's domains and the captcha vendors its sign-in uses.
const DEFAULT_ALLOW: &[&str] = &[
    "live.com",
    "microsoft.com",
    "microsoftonline.com",
    "msftauth.net",
    "msauth.net",
    "msecnd.net",
    "gfx.ms",
    "xboxlive.com",
    "xbox.com",
    "office.com",
    "office365.com",
    "outlook.com",
    "skype.com",
    "msn.com",
    "bing.com",
    // Captchas and bot checks the sign-in and the sign-up pages load (Arkose Labs, HUMAN/PerimeterX).
    "arkoselabs.com",
    "funcaptcha.com",
    "hcaptcha.com",
    "hsprotect.net",
    "px-cloud.net",
    "px-cdn.net",
    "perimeterx.net",
    "pxchk.net",
];

pub struct Policy {
    allow: Vec<String>,
}

impl Policy {
    /// The defaults, plus the domains in `XAU_WISP_ALLOW` (comma separated).
    pub fn from_env() -> Arc<Self> {
        let mut allow: Vec<String> = DEFAULT_ALLOW.iter().map(|s| (*s).to_string()).collect();
        if let Ok(extra) = std::env::var("XAU_WISP_ALLOW") {
            allow.extend(extra.split(',').map(|d| d.trim().trim_start_matches('.').to_ascii_lowercase()).filter(|d| d.contains('.')));
        }
        Arc::new(Self { allow })
    }

    /// Whether a stream to `host:port` may be opened.
    pub fn allows(&self, host: &str, port: u16) -> bool {
        if port != 443 && port != 80 {
            return false;
        }
        let host = host.trim_end_matches('.');
        // A name, never an address: the allow-list is about names.
        if host.is_empty() || host.len() > 253 || !host.contains('.') || host.parse::<std::net::IpAddr>().is_ok() {
            return false;
        }
        if !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_') {
            return false;
        }
        self.allow.iter().any(|d| host == d || host.strip_suffix(d.as_str()).is_some_and(|rest| rest.ends_with('.')))
    }
}

fn packet(kind: u8, id: u32, payload: &[u8]) -> Message {
    let mut v = Vec::with_capacity(5 + payload.len());
    v.push(kind);
    v.extend_from_slice(&id.to_le_bytes());
    v.extend_from_slice(payload);
    Message::Binary(v)
}

fn close_packet(id: u32, reason: u8) -> Message {
    packet(T_CLOSE, id, &[reason])
}

fn continue_packet(id: u32, remaining: u32) -> Message {
    packet(T_CONTINUE, id, &remaining.to_le_bytes())
}

/// A SOCKS5 connection to `host:port` through the proxy (`socks5h://addr:port`: the proxy
/// resolves the name, so no DNS question leaves this host).
async fn socks5(proxy: &str, host: &str, port: u16) -> io::Result<TcpStream> {
    let addr = proxy
        .strip_prefix("socks5h://")
        .or_else(|| proxy.strip_prefix("socks5://"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "o proxy de saída não é SOCKS5"))?;
    let mut s = TcpStream::connect(addr).await?;
    s.set_nodelay(true)?;
    s.write_all(&[5, 1, 0]).await?;
    let mut hello = [0u8; 2];
    s.read_exact(&mut hello).await?;
    if hello != [5, 0] {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "o proxy recusou o método de autenticação"));
    }
    let name = host.as_bytes();
    let len = u8::try_from(name.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "nome longo demais"))?;
    let mut req = vec![5, 1, 0, 3, len];
    req.extend_from_slice(name);
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "resposta SOCKS inválida"));
    }
    if head[1] != 0 {
        let kind = match head[1] {
            5 => io::ErrorKind::ConnectionRefused,
            6 => io::ErrorKind::TimedOut,
            _ => io::ErrorKind::Other,
        };
        return Err(io::Error::new(kind, format!("o proxy não conectou (código {})", head[1])));
    }
    let tail = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut l = [0u8; 1];
            s.read_exact(&mut l).await?;
            usize::from(l[0]) + 2
        }
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "endereço SOCKS inválido")),
    };
    let mut skip = vec![0u8; tail];
    s.read_exact(&mut skip).await?;
    Ok(s)
}

/// Out through the egress proxy when there is one; straight (a computer in development, which
/// has said so) when there is not.
async fn connect(proxy: Option<&str>, host: &str, port: u16) -> io::Result<TcpStream> {
    let tcp = match proxy {
        Some(p) => socks5(p, host, port).await?,
        None => TcpStream::connect((host, port)).await?,
    };
    let _ = tcp.set_nodelay(true);
    Ok(tcp)
}

fn reason_of(e: &io::Error) -> u8 {
    match e.kind() {
        io::ErrorKind::ConnectionRefused => R_REFUSED,
        io::ErrorKind::TimedOut => R_NO_RESPONSE,
        io::ErrorKind::NotFound | io::ErrorKind::AddrNotAvailable => R_UNREACHABLE,
        _ => R_NETWORK,
    }
}

struct Stream {
    data: mpsc::Sender<Vec<u8>>,
    task: tokio::task::JoinHandle<()>,
}

/// One stream: connect, then move bytes both ways until either side is done.
async fn run_stream(
    id: u32,
    host: String,
    port: u16,
    proxy: Option<String>,
    mut from_browser: mpsc::Receiver<Vec<u8>>,
    to_browser: mpsc::Sender<Message>,
    done: mpsc::UnboundedSender<u32>,
) {
    let started = std::time::Instant::now();
    let tcp = match tokio::time::timeout(CONNECT_TIMEOUT, connect(proxy.as_deref(), &host, port)).await {
        Ok(Ok(t)) => {
            tracing::debug!("wisp: #{id} {host}:{port} conectado em {} ms", started.elapsed().as_millis());
            t
        }
        Ok(Err(e)) => {
            tracing::info!("wisp: {host}:{port}: {e}");
            let _ = to_browser.send(close_packet(id, reason_of(&e))).await;
            let _ = done.send(id);
            return;
        }
        Err(_) => {
            tracing::info!("wisp: {host}:{port}: sem resposta em {}s", CONNECT_TIMEOUT.as_secs());
            let _ = to_browser.send(close_packet(id, R_NO_RESPONSE)).await;
            let _ = done.send(id);
            return;
        }
    };
    let (mut rd, mut wr) = tcp.into_split();
    let (touch, mut touched) = mpsc::channel::<()>(1);

    let out = to_browser.clone();
    let touch_rd = touch.clone();
    let mut reader = tokio::spawn(async move {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            match rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let _ = touch_rd.try_send(());
                    if out.send(packet(T_DATA, id, &buf[..n])).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    let out = to_browser.clone();
    let mut writer = tokio::spawn(async move {
        let mut written = 0u32;
        while let Some(chunk) = from_browser.recv().await {
            let _ = touch.try_send(());
            if wr.write_all(&chunk).await.is_err() {
                break;
            }
            written += 1;
            // The browser may send BUFFER packets ahead; say so again every half of them.
            if written % (BUFFER / 2) == 0 {
                let left = BUFFER.saturating_sub(u32::try_from(from_browser.len()).unwrap_or(BUFFER));
                if out.send(continue_packet(id, left)).await.is_err() {
                    break;
                }
            }
        }
        let _ = wr.shutdown().await;
    });

    loop {
        tokio::select! {
            _ = &mut reader => break,
            _ = &mut writer => break,
            t = tokio::time::timeout(IDLE_CLOSE, touched.recv()) => {
                if t.is_err() {
                    tracing::debug!("wisp: #{id} {host}:{port} parado há {}s; fechando", IDLE_CLOSE.as_secs());
                    break;
                }
            }
        }
    }
    reader.abort();
    writer.abort();
    tracing::debug!("wisp: #{id} {host}:{port} terminou após {} ms", started.elapsed().as_millis());
    let _ = to_browser.send(close_packet(id, R_VOLUNTARY)).await;
    let _ = done.send(id);
}

struct Guard;

impl Guard {
    fn take() -> Option<Self> {
        if SESSIONS.fetch_add(1, Ordering::SeqCst) >= MAX_SESSIONS {
            SESSIONS.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Guard)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        SESSIONS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Serve one browser connection until it closes.
pub async fn serve(socket: WebSocket, egress: Arc<Egress>, policy: Arc<Policy>) {
    let Some(_guard) = Guard::take() else {
        tracing::warn!("wisp: conexões demais; recusando mais uma");
        return;
    };
    let (mut sink, mut incoming) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Message>(256);
    let writer = tokio::spawn(async move {
        while let Some(m) = out_rx.recv().await {
            if sink.send(m).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    // The server speaks first.
    if out_tx.send(continue_packet(0, BUFFER)).await.is_err() {
        return;
    }

    tracing::debug!("wisp: conexão aberta");
    let proxy = egress.proxy().map(str::to_string);
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<u32>();
    let mut streams: HashMap<u32, Stream> = HashMap::new();
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;

    loop {
        tokio::select! {
            msg = incoming.next() => {
                let Some(Ok(msg)) = msg else { break };
                let data = match msg {
                    Message::Binary(b) => b,
                    Message::Close(_) => break,
                    _ => continue,
                };
                if data.len() < 5 {
                    continue;
                }
                let kind = data[0];
                let id = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                let payload = &data[5..];
                match kind {
                    T_CONNECT => {
                        let refuse = |reason: u8| out_tx.try_send(close_packet(id, reason));
                        if id == 0 || streams.contains_key(&id) || payload.len() < 3 {
                            let _ = refuse(R_INVALID);
                            continue;
                        }
                        if streams.len() >= MAX_STREAMS {
                            let _ = refuse(R_THROTTLED);
                            continue;
                        }
                        let port = u16::from_le_bytes([payload[1], payload[2]]);
                        let host = String::from_utf8_lossy(&payload[3..]).trim().to_ascii_lowercase();
                        if payload[0] != 1 || !policy.allows(&host, port) {
                            tracing::info!("wisp: recusado {host}:{port} (tipo {})", payload[0]);
                            let _ = refuse(R_BLOCKED);
                            continue;
                        }
                        // Fail closed: nothing goes out while the exit is not confirmed as Cloudflare's.
                        if let Err(e) = egress.require() {
                            tracing::info!("wisp: {host}:{port} recusado: {e}");
                            let _ = refuse(R_NETWORK);
                            continue;
                        }
                        let (tx, rx) = mpsc::channel::<Vec<u8>>(512);
                        let task = tokio::spawn(run_stream(id, host, port, proxy.clone(), rx, out_tx.clone(), done_tx.clone()));
                        streams.insert(id, Stream { data: tx, task });
                    }
                    T_DATA => {
                        if let Some(s) = streams.get(&id) {
                            if s.data.try_send(payload.to_vec()).is_err() {
                                // More than it was allowed to send ahead: it is not following the flow control.
                                if let Some(s) = streams.remove(&id) {
                                    s.task.abort();
                                }
                                let _ = out_tx.try_send(close_packet(id, R_THROTTLED));
                            }
                        }
                    }
                    T_CLOSE => {
                        if let Some(s) = streams.remove(&id) {
                            s.task.abort();
                        }
                    }
                    _ => {}
                }
            }
            Some(id) = done_rx.recv() => {
                streams.remove(&id);
            }
            _ = ping.tick() => {
                if out_tx.try_send(Message::Ping(Vec::new())).is_err() && out_tx.is_closed() {
                    break;
                }
            }
        }
    }

    tracing::debug!("wisp: conexão fechada ({} streams abertos)", streams.len());
    for (_, s) in streams {
        s.task.abort();
    }
    drop(out_tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn policy() -> Policy {
        Policy { allow: DEFAULT_ALLOW.iter().map(|s| (*s).to_string()).collect() }
    }

    #[test]
    fn only_microsofts_names_on_web_ports_are_allowed() {
        let p = policy();
        assert!(p.allows("login.live.com", 443));
        assert!(p.allows("logincdn.msftauth.net", 443));
        assert!(p.allows("LOGIN.LIVE.COM.".to_ascii_lowercase().as_str(), 443));
        assert!(p.allows("signup.live.com", 80));
        assert!(p.allows("live.com", 443));
        assert!(!p.allows("login.live.com", 22), "not a web port");
        assert!(!p.allows("login.live.com", 8080));
        assert!(!p.allows("evil.com", 443));
        assert!(!p.allows("notlive.com", 443), "a suffix of the text is not a subdomain");
        assert!(!p.allows("live.com.evil.com", 443));
        assert!(!p.allows("127.0.0.1", 443), "an address is not a name");
        assert!(!p.allows("10.0.0.5", 80));
        assert!(!p.allows("[::1]", 443));
        assert!(!p.allows("localhost", 443));
        assert!(!p.allows("", 443));
        assert!(!p.allows("login.live.com/evil", 443));
        assert!(!p.allows("login.live.com@evil.com", 443));
    }

    #[test]
    fn packets_have_the_wisp_layout() {
        let Message::Binary(b) = packet(T_DATA, 7, b"hi") else { panic!() };
        assert_eq!(b, [2, 7, 0, 0, 0, b'h', b'i']);
        let Message::Binary(b) = continue_packet(0, 128) else { panic!() };
        assert_eq!(b, [3, 0, 0, 0, 0, 128, 0, 0, 0]);
        let Message::Binary(b) = close_packet(9, R_BLOCKED) else { panic!() };
        assert_eq!(b, [4, 9, 0, 0, 0, 0x48]);
    }

    /// A SOCKS5 proxy that answers "connected" and then echoes, to check what we ask of it.
    async fn fake_proxy() -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let h = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut hello = [0u8; 3];
            s.read_exact(&mut hello).await.unwrap();
            assert_eq!(hello, [5, 1, 0]);
            s.write_all(&[5, 0]).await.unwrap();
            let mut head = [0u8; 5];
            s.read_exact(&mut head).await.unwrap();
            assert_eq!(&head[..4], &[5, 1, 0, 3], "a CONNECT by name, never by address");
            let mut rest = vec![0u8; usize::from(head[4]) + 2];
            s.read_exact(&mut rest).await.unwrap();
            s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
            let mut buf = [0u8; 64];
            let n = s.read(&mut buf).await.unwrap();
            s.write_all(&buf[..n]).await.unwrap();
            rest
        });
        (format!("socks5h://{addr}"), h)
    }

    #[tokio::test]
    async fn a_stream_asks_the_proxy_for_the_name_and_relays_both_ways() {
        let (proxy, asked) = fake_proxy().await;
        let (data_tx, data_rx) = mpsc::channel(8);
        let (out_tx, mut out_rx) = mpsc::channel(8);
        let (done_tx, mut done_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run_stream(5, "login.live.com".into(), 443, Some(proxy), data_rx, out_tx, done_tx));
        data_tx.send(b"ping".to_vec()).await.unwrap();
        let Some(Message::Binary(b)) = out_rx.recv().await else { panic!("no data back") };
        assert_eq!(b, [2, 5, 0, 0, 0, b'p', b'i', b'n', b'g']);
        let rest = asked.await.unwrap();
        assert_eq!(&rest[..rest.len() - 2], b"login.live.com");
        assert_eq!(&rest[rest.len() - 2..], &443u16.to_be_bytes());
        drop(data_tx);
        task.abort();
        let _ = done_rx.try_recv();
    }

    #[tokio::test]
    async fn a_proxy_that_cannot_connect_closes_the_stream_with_a_reason() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = [0u8; 64];
            let _ = s.read(&mut buf).await;
            s.write_all(&[5, 0]).await.unwrap();
            let _ = s.read(&mut buf).await;
            s.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
        });
        let (_data_tx, data_rx) = mpsc::channel(8);
        let (out_tx, mut out_rx) = mpsc::channel(8);
        let (done_tx, mut done_rx) = mpsc::unbounded_channel();
        tokio::spawn(run_stream(3, "login.live.com".into(), 443, Some(format!("socks5h://{addr}")), data_rx, out_tx, done_tx));
        let Some(Message::Binary(b)) = out_rx.recv().await else { panic!("no close") };
        assert_eq!(b, [4, 3, 0, 0, 0, R_REFUSED]);
        assert_eq!(done_rx.recv().await, Some(3));
    }

    #[tokio::test]
    async fn the_proxy_must_be_socks5() {
        let e = connect(Some("http://127.0.0.1:1"), "login.live.com", 443).await.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Unsupported);
    }
}
