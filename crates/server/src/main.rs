mod api;
mod assets;
mod guard;
mod terminal;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::{Router, middleware, routing::get};
use clap::Parser;
use fastssh_core::Store;
use tokio::net::TcpListener;

/// FastSSH server: serves the web interface and the terminal sessions behind it.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Address to listen on. Keep this on localhost until logins exist:
    /// anyone who can reach it gets a shell as you.
    #[arg(long, env = "FASTSSH_LISTEN", default_value = "127.0.0.1:7422")]
    listen: SocketAddr,

    /// Where the database lives. Defaults to the per-user data folder
    /// (~/.local/share/fastssh on Linux).
    #[arg(long, env = "FASTSSH_DATA_DIR")]
    data_dir: Option<PathBuf>,
}

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
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
    let local_only = args.listen.ip().is_loopback();
    if !local_only {
        tracing::warn!(
            "listening on {} with no authentication: anyone who can reach this port gets a shell",
            args.listen
        );
    }

    let data_dir = match args.data_dir {
        Some(dir) => dir,
        None => dirs::data_dir()
            .context("no data folder for this user; pass --data-dir")?
            .join("fastssh"),
    };
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating {}", data_dir.display()))?;
    let db_path = data_dir.join("fastssh.db");
    let store = Store::open(&db_path).with_context(|| format!("opening {}", db_path.display()))?;

    let app = Router::new()
        .route("/ws", get(terminal::upgrade))
        .nest("/api", api::router())
        .fallback(assets::serve)
        .layer(middleware::from_fn_with_state(
            guard::Policy { local_only },
            guard::check,
        ))
        .with_state(AppState { store });

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
