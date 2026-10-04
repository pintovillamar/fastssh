mod api;
mod assets;
mod auth;
mod error;
mod google;
mod guard;
mod terminal;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::{Router, http::HeaderMap, http::header, middleware, routing::get};
use clap::Parser;
use fastssh_core::{Store, store::User};
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

pub struct Config {
    pub allow_signup: bool,
    pub local_shell: bool,
    pub secure_cookies: bool,
    pub google: Option<google::Google>,
    /// `--public-url` without a trailing slash.
    public_base: Option<String>,
}

impl Config {
    /// The local shell runs as the server's own OS user, so it is for the
    /// admin only.
    pub fn local_shell_for(&self, user: &User) -> bool {
        self.local_shell && user.is_admin
    }

    /// Where the browser reaches us, e.g. `https://ssh.example.com`. Without
    /// `--public-url` we are on localhost (startup checks this when it
    /// matters) and use whichever local name the browser used.
    pub fn base_url(&self, headers: &HeaderMap) -> String {
        match &self.public_base {
            Some(base) => base.clone(),
            None => {
                let host = headers
                    .get(header::HOST)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("localhost");
                format!("http://{host}")
            }
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub config: Arc<Config>,
    pub keys: Arc<auth::Keys>,
    pub limiter: Arc<auth::Limiter>,
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

    let google = match (args.google_client_id, args.google_client_secret) {
        (Some(id), Some(secret)) => {
            if !local_only && args.public_url.is_none() {
                bail!("Google sign-in needs --public-url so Google knows where to send people back");
            }
            let google = google::Google::new(id, secret);
            url::Url::parse(&google.auth_url).context("FASTSSH_GOOGLE_AUTH_URL")?;
            Some(google)
        }
        _ => None,
    };

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

    let policy = guard::Policy {
        local_only,
        public_host: args.public_url.as_ref().and_then(|url| {
            let host = url.host_str()?;
            Some(match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_owned(),
            })
        }),
    };
    let state = AppState {
        store,
        config: Arc::new(Config {
            allow_signup: args.allow_signup,
            local_shell: !args.no_local_shell,
            secure_cookies: https,
            google,
            public_base: args
                .public_url
                .map(|url| url.as_str().trim_end_matches('/').to_owned()),
        }),
        keys: Arc::default(),
        limiter: Arc::default(),
    };

    let app = Router::new()
        .route("/ws", get(terminal::upgrade))
        .nest(
            "/api",
            Router::new()
                .merge(auth::router())
                .merge(google::router())
                .merge(api::router()),
        )
        .fallback(assets::serve)
        .layer(middleware::from_fn_with_state(policy, guard::check))
        .with_state(state);

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
