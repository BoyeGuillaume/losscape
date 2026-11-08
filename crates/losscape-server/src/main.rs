use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use losscape_server::app;
use losscape_server::config::{self, Config};
use losscape_server::state::AppState;
use tracing_subscriber::EnvFilter;

/// Stopping normally takes moments. If something still hangs after this, the server exits
/// anyway: what isn't stored by then was never acknowledged, so the SDKs send it again.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(25);

const USAGE: &str = "\
losscape server

Usage: losscape-server [--config <file>] [healthcheck]

  --config <file>  Settings from a TOML file, for what the environment doesn't set
                   (also LOSSCAPE_CONFIG). See docs/DEPLOYMENT.md
  healthcheck      Ask a running server whether it's ready: exit status 0 if it is
  --version        Print the version
";

enum Command {
    Serve,
    Healthcheck,
}

fn parse_args() -> Result<Option<(Command, Option<PathBuf>)>, String> {
    let mut command = Command::Serve;
    let mut file = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => file = Some(args.next().ok_or("--config needs a file")?.into()),
            "healthcheck" => command = Command::Healthcheck,
            "--version" | "-V" => {
                println!("losscape-server {}", losscape_core::VERSION);
                return Ok(None);
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                return Ok(None);
            }
            other => return Err(format!("unknown argument {other:?}\n\n{USAGE}")),
        }
    }
    Ok(Some((command, file)))
}

fn main() -> ExitCode {
    dotenvy::dotenv().ok();
    let (command, file) = match parse_args() {
        Ok(Some(parsed)) => parsed,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let result = match command {
        Command::Healthcheck => healthcheck(file),
        Command::Serve => tokio::runtime::Runtime::new()
            .map_err(Into::into)
            .and_then(|runtime| runtime.block_on(serve(file))),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("losscape-server: {err}");
            ExitCode::FAILURE
        }
    }
}

type Error = Box<dyn std::error::Error + Send + Sync>;

/// For container health checks, whose images have no curl: `GET /readyz` on this machine.
fn healthcheck(file: Option<PathBuf>) -> Result<(), Error> {
    let listen = config::listen_address(file.as_deref())?;
    let host = match listen.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    let timeout = Duration::from_secs(3);
    let mut stream = TcpStream::connect_timeout(&SocketAddr::new(host, listen.port()), timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(b"GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status = response.lines().next().unwrap_or_default();
    if status.split_whitespace().nth(1) == Some("200") {
        Ok(())
    } else {
        let reason = response.rsplit("\r\n\r\n").next().unwrap_or_default();
        Err(format!("not ready: {status} {reason}").into())
    }
}

async fn serve(file: Option<PathBuf>) -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::load(file.as_deref())?;
    let (state, flusher) = AppState::start(&config).await?;
    let live = state.live().clone();
    let purger = losscape_server::purge::spawn_purger(state.clone());
    let backfill = state.clone();
    let backfiller = tokio::spawn(async move {
        match losscape_server::purge::backfill_last_step(&backfill).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(runs = n, "filled in step counts of older runs"),
            Err(err) => tracing::warn!(%err.message, "filling in step counts failed"),
        }
        match losscape_server::purge::backfill_metric_index(&backfill).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(runs = n, "filled in metric ranges of older runs"),
            Err(err) => tracing::warn!(%err.message, "filling in metric ranges failed"),
        }
    });

    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    tracing::info!(addr = %config.listen, version = losscape_core::VERSION, "losscape server listening");
    let auth = &config.auth;
    tracing::info!(
        signup = ?auth.signup,
        oauth = ?auth.oauth.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        public_url = auth.public_url.as_deref().unwrap_or("-"),
        trusted_proxies = auth.trusted_proxies.len(),
        "accounts"
    );
    match &config.web_dir {
        Some(dir) => tracing::info!(dir = %dir.display(), "serving web UI"),
        None => tracing::info!("web UI not built (cd web && npm run build); API only"),
    }
    let app = app::router(state, config.max_body_bytes, config.web_dir.as_deref());
    // Connect info gives the login rate limiter the client address.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        // Live streams never end by themselves, and connections are waited for.
        live.shutdown();
        tokio::spawn(async {
            tokio::time::sleep(SHUTDOWN_LIMIT).await;
            tracing::warn!("still stopping after {SHUTDOWN_LIMIT:?}: exiting anyway");
            std::process::exit(1);
        });
    })
    .await?;

    // The flusher ends once nothing can queue any more: these two hold the state as well.
    purger.abort();
    backfiller.abort();
    // The router (and its queue handles) is gone; let the flusher write what is queued.
    flusher.await?;
    tracing::info!("stopped");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutting down");
}
