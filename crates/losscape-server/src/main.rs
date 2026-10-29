use losscape_server::app;
use losscape_server::config::Config;
use losscape_server::state::AppState;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    let (state, flusher) = AppState::start(&config).await?;

    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    tracing::info!(addr = %config.listen, version = losscape_core::VERSION, "losscape server listening");
    axum::serve(listener, app::router(state, config.max_body_bytes))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

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
