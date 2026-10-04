//! The FastSSH server as a library.
//!
//! The `fastssh` command wraps this with command-line options. The desktop
//! app embeds it and points its window at it, which is how both share one
//! interface.

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
use fastssh_core::{LocalShell, Store, store::User};
use tokio::net::TcpListener;

/// Everything that decides how a server behaves.
pub struct Options {
    /// Folder that holds `fastssh.db`. Created if missing.
    pub data_dir: PathBuf,
    /// Whether the listener is only reachable from this machine.
    pub local_only: bool,
    /// The address people use to reach the server, e.g. https://ssh.example.com.
    pub public_url: Option<url::Url>,
    pub allow_signup: bool,
    /// Offer the admin a shell on this machine.
    pub local_shell: bool,
    /// Google OAuth client ID and secret.
    pub google: Option<(String, String)>,
    /// Running inside the desktop app: one local profile protected by a
    /// master password, with no email address or sign-ups.
    pub desktop: bool,
}

pub(crate) struct Config {
    pub allow_signup: bool,
    pub local_shell: bool,
    pub secure_cookies: bool,
    pub desktop: bool,
    pub google: Option<google::Google>,
    /// `public_url` without a trailing slash.
    public_base: Option<String>,
}

impl Config {
    /// The local shell runs as the server's own OS user, so it is for the
    /// admin only.
    pub fn local_shell_for(&self, user: &User) -> bool {
        self.local_shell && user.is_admin
    }

    /// Where the browser reaches us, e.g. `https://ssh.example.com`. Without
    /// a public URL we are on localhost (checked in [`app`] when it matters)
    /// and use whichever local name the browser used.
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
pub(crate) struct AppState {
    pub store: Store,
    pub config: Arc<Config>,
    pub keys: Arc<auth::Keys>,
    pub limiter: Arc<auth::Limiter>,
}

/// Opens the database and builds the whole application.
pub fn app(options: Options) -> Result<Router> {
    let https = options.public_url.as_ref().is_some_and(|url| url.scheme() == "https");

    let google = match options.google {
        Some((id, secret)) => {
            if !options.local_only && options.public_url.is_none() {
                bail!("Google sign-in needs a public URL so Google knows where to send people back");
            }
            let google = google::Google::new(id, secret);
            url::Url::parse(&google.auth_url).context("FASTSSH_GOOGLE_AUTH_URL")?;
            Some(google)
        }
        None => None,
    };

    std::fs::create_dir_all(&options.data_dir)
        .with_context(|| format!("creating {}", options.data_dir.display()))?;
    let db_path = options.data_dir.join("fastssh.db");
    restrict_to_owner(&db_path).with_context(|| format!("preparing {}", db_path.display()))?;
    let store = Store::open(&db_path).with_context(|| format!("opening {}", db_path.display()))?;

    let policy = guard::Policy {
        local_only: options.local_only,
        public_host: options.public_url.as_ref().and_then(|url| {
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
            // The desktop app has exactly one profile.
            allow_signup: options.allow_signup && !options.desktop,
            // Off on platforms where the local shell is not built yet.
            local_shell: options.local_shell && LocalShell::SUPPORTED,
            secure_cookies: https,
            desktop: options.desktop,
            google,
            public_base: options
                .public_url
                .map(|url| url.as_str().trim_end_matches('/').to_owned()),
        }),
        keys: Arc::default(),
        limiter: Arc::default(),
    };

    Ok(Router::new()
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
        .with_state(state))
}

/// Serves `app` on `listener` until the task is dropped or the listener fails.
pub async fn serve(listener: TcpListener, app: Router) -> Result<()> {
    axum::serve(listener, app).await?;
    Ok(())
}

/// Starts a server for the desktop app on a free localhost port and returns
/// its address. Must be called from inside a Tokio runtime; the server runs
/// until the process exits.
pub fn spawn_local(options: Options) -> Result<SocketAddr> {
    let app = app(options)?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").context("binding a local port")?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?;
    let listener = TcpListener::from_std(listener)?;
    tokio::spawn(async move {
        if let Err(err) = serve(listener, app).await {
            tracing::error!("server stopped: {err:#}");
        }
    });
    Ok(address)
}

/// Makes sure the database file exists and only its owner can read it. It
/// holds email addresses, host names and (encrypted) secrets. SQLite gives
/// its side files the same permissions as the main file.
fn restrict_to_owner(path: &std::path::Path) -> std::io::Result<()> {
    let file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    drop(file);
    Ok(())
}
