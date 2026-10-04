use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use fastssh::Options;
use tokio::net::TcpListener;

/// FastSSH server: serves the web interface and the terminal sessions behind it.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Address to listen on.
    #[arg(long, env = "FASTSSH_LISTEN", default_value = "127.0.0.1:7422")]
    listen: SocketAddr,

    /// Where the database lives. Defaults to the per-user data folder
    /// (~/.local/share/fastssh on Linux).
    #[arg(long, env = "FASTSSH_DATA_DIR")]
    data_dir: Option<PathBuf>,

    /// The address people type to reach this server, e.g.
    /// https://ssh.example.com. Needed behind a reverse proxy and for Google
    /// sign-in. With https, cookies are marked Secure.
    #[arg(long, env = "FASTSSH_PUBLIC_URL")]
    public_url: Option<url::Url>,

    /// Let anyone who can reach the server create an account. Without this,
    /// only the first account (the admin) can be created.
    #[arg(long, env = "FASTSSH_ALLOW_SIGNUP")]
    allow_signup: bool,

    /// Do not offer the admin a shell on the machine running FastSSH.
    #[arg(long, env = "FASTSSH_NO_LOCAL_SHELL")]
    no_local_shell: bool,

    /// Google OAuth client ID. Together with the secret, turns on
    /// "Sign in with Google".
    #[arg(long, env = "FASTSSH_GOOGLE_CLIENT_ID", requires = "google_client_secret")]
    google_client_id: Option<String>,

    /// Google OAuth client secret. Prefer the environment variable, so it
    /// does not show up in the process list.
    #[arg(long, env = "FASTSSH_GOOGLE_CLIENT_SECRET", requires = "google_client_id", hide_env_values = true)]
    google_client_secret: Option<String>,
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
    let https = args.public_url.as_ref().is_some_and(|url| url.scheme() == "https");
    if !local_only && !https {
        tracing::warn!(
            "listening on {} without an https --public-url: passwords and terminal \
             sessions will cross the network unencrypted",
            args.listen
        );
    }

    let data_dir = match args.data_dir {
        Some(dir) => dir,
        None => dirs::data_dir()
            .context("no data folder for this user; pass --data-dir")?
            .join("fastssh"),
    };
    let app = fastssh::app(Options {
        data_dir,
        local_only,
        public_url: args.public_url,
        allow_signup: args.allow_signup,
        local_shell: !args.no_local_shell,
        google: args.google_client_id.zip(args.google_client_secret),
        desktop: false,
    })?;

    let listener = TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("binding {}", args.listen))?;
    tracing::info!("FastSSH ready at http://{}", args.listen);

    // Open terminals would keep a "graceful" shutdown waiting forever, so on a
    // stop signal the server simply ends; SQLite is safe to stop at any point.
    tokio::select! {
        result = fastssh::serve(listener, app) => result?,
        () = stop_signal() => tracing::info!("stopping"),
    }
    Ok(())
}

/// Completes on Ctrl+C or on SIGTERM, which is what systemd and Docker send.
async fn stop_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}
