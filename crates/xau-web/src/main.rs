//! The web version of the app.
//!
//! A local server that runs the exact core the Android app runs (sessions,
//! freeze, farm, store, offers) and serves a browser UI for it. A page
//! cannot talk to Xbox Live itself, so the server does, and the browser is
//! only the screen. Run it, open the link it prints, and everything the app
//! does is there.

mod assets;
mod auth;
mod cdphost;
mod egress;
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
accounts, so keep both private.

As a server (a cloud host), it is set by the environment, not the options:

  PORT                the port to listen on, on every interface (the host sets it)
  XAU_PUBLIC_HOSTS    the domain names it answers to, comma separated (turns on server mode)
  XAU_PASSWORD        the password(s) the front page asks for: one, or several separated
                      by commas, any of which opens the site (4+ characters each)
  XAU_KEY             the session key (32+ characters); else one is made and kept in the data folder
  XAU_DATA_DIR        where accounts and codes are kept (a persistent volume)
  XAU_CHROMIUM        the browser that runs Microsoft's sign-in page (default: the usual places)";

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

    // On a server the environment says where it listens and what it is called.
    let public_hosts: Vec<String> = std::env::var("XAU_PUBLIC_HOSTS")
        .unwrap_or_default()
        .split(',')
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .collect();
    let env_port: Option<u16> = std::env::var("PORT").ok().and_then(|p| p.parse().ok());
    let server_mode = env_port.is_some() || !public_hosts.is_empty();
    let args = Args { port: env_port.unwrap_or(args.port), open: args.open && !server_mode, ..args };

    let dir = args
        .data_dir
        .clone()
        .or_else(|| std::env::var_os("XAU_DATA_DIR").map(PathBuf::from))
        .unwrap_or_else(default_data_dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("não foi possível criar {}: {e}", dir.display());
        std::process::exit(1);
    }
    xau_mobile::accounts::set_data_dir(dir.clone());
    let password = match auth::passwords_from(&std::env::var("XAU_PASSWORD").unwrap_or_default()) {
        Ok(list) => Some(list).filter(|l| !l.is_empty()),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let key_from_env = auth::key_from_env();
    let key = match key_from_env.clone().map_or_else(|| auth::load_or_create_key(&dir), Ok) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("não foi possível guardar a chave em {}: {e}", dir.display());
            std::process::exit(1);
        }
    };

    // Everything for Microsoft leaves through Cloudflare (WARP) when so configured, set before any
    // other thread exists: the HTTP clients read it from the environment.
    let egress = egress::Egress::from_env(server_mode);
    egress.apply_to_environment();

    // The windows need the main thread; the server gets its own.
    let (host, event_loop) = webhost::create();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("the async runtime");

    let bind_ip = if args.lan || server_mode { [0, 0, 0, 0] } else { [127, 0, 0, 1] };
    let listener = runtime.block_on(async {
        // A server listens where it was told, and nowhere else; a computer takes the next free port.
        let last = if server_mode { args.port } else { args.port.saturating_add(9) };
        for port in args.port..=last {
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
        access: auth::Access { lan: args.lan, hosts: Arc::new(public_hosts.clone()) },
        host,
        egress: egress.clone(),
        password: password.clone().map(Arc::new),
        throttle: Arc::default(),
        logins: Arc::default(),
        checkout: Arc::default(),
    };
    let router = server::router(app);

    println!("xau-web {} — a versão web do app", env!("CARGO_PKG_VERSION"));
    println!("  dados:    {}", dir.display());
    if server_mode {
        println!("  saída:    {}", egress_note(&std::env::var("XAU_EGRESS_PROXY").unwrap_or_default()));
        println!("  servidor: porta {port}, responde a: {}", if public_hosts.is_empty() { "localhost".to_string() } else { public_hosts.join(", ") });
        if let Some(list) = &password {
            let which = if list.len() == 1 { "a senha".to_string() } else { format!("qualquer uma das {} senhas", list.len()) };
            println!("  acesso:   {which} (XAU_PASSWORD), pedida na página inicial; tentativas limitadas");
        } else if key_from_env.is_some() {
            println!("  chave:    a de XAU_KEY (abra https://<domínio>/?key=<a chave> uma vez)");
        } else {
            println!("  chave:    {key}  (abra https://<domínio>/?key=<a chave> uma vez; defina XAU_KEY para não depender dos logs)");
        }
    } else {
        let url = format!("http://localhost:{port}/?key={key}");
        println!("  abra:     {url}");
        if args.lan {
            println!("  na rede:  http://<ip deste computador>:{port}/?key={key}");
        }
        println!("  (o link traz a chave: quem o tiver controla as contas)");
        if args.open {
            open_browser(&url);
        }
    }
    std::thread::spawn(move || {
        runtime.block_on(async move {
            tokio::spawn(egress.watch());
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
fn egress_note(proxy: &str) -> String {
    if proxy.is_empty() {
        "SEM proxy: as chamadas à Microsoft ficam recusadas (configure o WARP, ou XAU_ALLOW_DIRECT_EGRESS=1 para testar)".into()
    } else {
        "tudo para a Microsoft sai pela Cloudflare (WARP), confirmado antes de cada uso".into()
    }
}