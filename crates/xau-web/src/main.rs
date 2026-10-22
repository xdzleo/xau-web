//! The web version of the app.
//!
//! A local server that runs the exact core the Android app runs (sessions,
//! freeze, farm, store, offers) and serves a browser UI for it. A page
//! cannot talk to Xbox Live itself, so the server does, and the browser is
//! only the screen. Run it, open the link it prints, and everything the app
//! does is there.

mod assets;
mod auth;
mod profiles;
mod server;
mod signin;
mod webhost;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

const HELP: &str = "xau-web: the web version of the app

USAGE: xau-web [OPTIONS]

  --port <n>        port to listen on (default 8787; the next free one if taken)
  --data-dir <dir>  where accounts and codes are kept (default: the user data folder)
  --lan             listen on the local network, not only on this computer
  --no-open         do not open the browser
  -h, --help        this text

The link it prints carries a key that only this installation knows. Anyone
who has that link, or the web-key.txt in the data folder, controls the
accounts, so keep both private.";

struct Args {
    port: u16,
    data_dir: Option<PathBuf>,
    lan: bool,
    open: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { port: 8787, data_dir: None, lan: false, open: true };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--port" => {
                a.port = it.next().and_then(|p| p.parse().ok()).ok_or("--port precisa de um número")?;
            }
            "--data-dir" => a.data_dir = Some(it.next().ok_or("--data-dir precisa de uma pasta")?.into()),
            "--lan" => a.lan = true,
            "--no-open" => a.open = false,
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            other => return Err(format!("opção desconhecida: {other}\n\n{HELP}")),
        }
    }
    Ok(a)
}

fn default_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("xau-web")
}

fn open_browser(url: &str) {
    #[cfg(windows)]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

fn main() {
    init_logging();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };

    let dir = args.data_dir.clone().unwrap_or_else(default_data_dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("não foi possível criar {}: {e}", dir.display());
        std::process::exit(1);
    }
    xau_mobile::accounts::set_data_dir(dir.clone());
    let key = match auth::load_or_create_key(&dir) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("não foi possível guardar a chave em {}: {e}", dir.display());
            std::process::exit(1);
        }
    };

    // The windows need the main thread; the server gets its own.
    let (host, event_loop) = webhost::create();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("the async runtime");

    let bind_ip = if args.lan { [0, 0, 0, 0] } else { [127, 0, 0, 1] };
    let listener = runtime.block_on(async {
        for port in args.port..args.port.saturating_add(10) {
            if let Ok(l) = tokio::net::TcpListener::bind(SocketAddr::from((bind_ip, port))).await {
                return Some(l);
            }
        }
        None
    });
    let Some(listener) = listener else {
        eprintln!("nenhuma porta livre a partir de {}", args.port);
        std::process::exit(1);
    };
    let port = listener.local_addr().map_or(args.port, |a| a.port());

    let app = server::App {
        key: Arc::new(key.clone()),
        port,
        lan: args.lan,
        host,
        logins: Arc::default(),
        checkout: Arc::default(),
    };
    let router = server::router(app);

    let url = format!("http://localhost:{port}/?key={key}");
    println!("xau-web {} — a versão web do app", env!("CARGO_PKG_VERSION"));
    println!("  dados:    {}", dir.display());
    println!("  abra:     {url}");
    if args.lan {
        println!("  na rede:  http://<ip deste computador>:{port}/?key={key}");
    }
    println!("  (o link traz a chave: quem o tiver controla as contas)");
    if args.open {
        open_browser(&url);
    }

    std::thread::spawn(move || {
        runtime.block_on(async move {
            if let Err(e) = axum::serve(listener, router).await {
                eprintln!("o servidor parou: {e}");
                std::process::exit(1);
            }
        });
    });

    // Never returns: the windows live here.
    event_loop.run()
}

fn init_logging() {
    // The core logs through `tracing`; without a subscriber a failing
    // sign-in leaves no trail at all.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(false).init();
}