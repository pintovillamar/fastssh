mod assets;
mod terminal;

use std::net::SocketAddr;

use anyhow::{Context, Result};
use axum::{Router, routing::get};
use clap::Parser;
use tokio::net::TcpListener;

/// FastSSH server: serves the web interface and the terminal sessions behind it.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Address to listen on. Keep this on localhost until logins exist:
    /// anyone who can reach it gets a shell as you.
    #[arg(long, env = "FASTSSH_LISTEN", default_value = "127.0.0.1:7422")]
    listen: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "fastssh=info".into()),
        )
        .init();

    let args = Args::parse();
    if !args.listen.ip().is_loopback() {
        tracing::warn!(
            "listening on {} with no authentication: anyone who can reach this port gets a shell",
            args.listen
        );
    }

    let app = Router::new()
        .route("/ws", get(terminal::upgrade))
        .fallback(assets::serve);

    let listener = TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("binding {}", args.listen))?;
    tracing::info!("FastSSH ready at http://{}", args.listen);

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
