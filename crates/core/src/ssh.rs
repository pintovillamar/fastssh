//! SSH sessions.
//!
//! Connecting happens in three steps so the caller can talk to the user in
//! between: [`connect`] (which may ask whether to trust the host key), one of
//! the `auth_*` methods, then [`SshClient::open_shell`].

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect, client};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

use crate::store::{SavedConnection, Store};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const OUTPUT_BACKLOG: usize = 64;

/// Sent to the caller the first time we see a server. Reply `true` to trust
/// the key and remember it.
pub struct HostKeyQuestion {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub reply: oneshot::Sender<bool>,
}

struct Handler {
    user_id: i64,
    host: String,
    port: u16,
    store: Store,
    questions: mpsc::Sender<HostKeyQuestion>,
}

impl client::Handler for Handler {
    type Error = anyhow::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool> {
        let key = key.public_key();
        let presented = key.to_openssh().context("encoding host key")?;
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();

        match self.store.known_host(self.user_id, &self.host, self.port)? {
            Some(trusted) if trusted == presented => return Ok(true),
            Some(_) => bail!(
                "the host key for {}:{} has changed (now {fingerprint}). This can mean someone \
                 is intercepting the connection. If the server was reinstalled, forget its saved \
                 key and connect again.",
                self.host,
                self.port
            ),
            None => {}
        }

        let (reply, answer) = oneshot::channel();
        let question = HostKeyQuestion {
            host: self.host.clone(),
            port: self.port,
            algorithm: key.algorithm().to_string(),
            fingerprint,
            reply,
        };
        let trusted = self.questions.send(question).await.is_ok() && answer.await.unwrap_or(false);
        if !trusted {
            bail!("host key not trusted");
        }
        self.store.trust_host(self.user_id, &self.host, self.port, &presented)?;
        Ok(true)
    }
}

/// A connected but not yet authenticated session.
pub struct SshClient {
    handle: client::Handle<Handler>,
    username: String,
}

/// Opens the connection and verifies the host key. An unknown key is sent to
/// `questions`; a key that differs from the trusted one is an error.
pub async fn connect(
    user_id: i64,
    connection: &SavedConnection,
    store: Store,
    questions: mpsc::Sender<HostKeyQuestion>,
) -> Result<SshClient> {
    let details = &connection.details;
    let address = (details.host.as_str(), details.port);
    // The timeout covers only the TCP connect, so a user taking their time
    // over the host key question is not cut off.
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address))
        .await
        .map_err(|_| anyhow!("timed out connecting to {}:{}", details.host, details.port))?
        .with_context(|| format!("connecting to {}:{}", details.host, details.port))?;
    stream.set_nodelay(true)?;

    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });
    let handler = Handler {
        user_id,
        host: details.host.clone(),
        port: details.port,
        store,
        questions,
    };
    let handle = client::connect_stream(config, stream, handler).await?;
    Ok(SshClient {
        handle,
        username: details.username.clone(),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("the key is protected by a passphrase")]
    NeedsPassphrase,
    #[error("not a usable private key: {0}")]
    Invalid(String),
}

/// Parses a private key (OpenSSH or PEM text), decrypting it if a passphrase
/// is given.
pub async fn decode_key(text: String, passphrase: Option<String>) -> Result<PrivateKey, KeyError> {
    // Decrypting a key runs a deliberately slow KDF; keep it off async threads.
    let decoded = tokio::task::spawn_blocking(move || {
        russh::keys::decode_secret_key(&text, passphrase.as_deref())
    })
    .await;
    match decoded {
        Ok(Ok(key)) => Ok(key),
        Ok(Err(russh::keys::Error::KeyIsEncrypted)) => Err(KeyError::NeedsPassphrase),
        Ok(Err(err)) => Err(KeyError::Invalid(err.to_string())),
        Err(err) => Err(KeyError::Invalid(err.to_string())),
    }
}

impl SshClient {
    /// Returns whether the server accepted the password.
    pub async fn auth_password(&mut self, password: String) -> Result<bool> {
        let result = self
            .handle
            .authenticate_password(self.username.clone(), password)
            .await?;
        Ok(result.success())
    }

    /// Returns whether the server accepted the key.
    pub async fn auth_key(&mut self, key: PrivateKey) -> Result<bool> {
        let hash = self.handle.best_supported_rsa_hash().await?.flatten();
        let result = self
            .handle
            .authenticate_publickey(
                self.username.clone(),
                PrivateKeyWithHashAlg::new(Arc::new(key), hash),
            )
            .await?;
        Ok(result.success())
    }

    /// Starts the remote login shell. The returned stream ends when the shell
    /// exits or the connection drops.
    pub async fn open_shell(self, cols: u16, rows: u16) -> Result<(SshShell, mpsc::Receiver<Vec<u8>>)> {
        let mut channel = self.handle.channel_open_session().await?;
        channel
            .request_pty(false, "xterm-256color", cols.into(), rows.into(), 0, 0, &[])
            .await?;
        channel.request_shell(false).await?;

        let (output_tx, output_rx) = mpsc::channel(OUTPUT_BACKLOG);
        let (command_tx, mut command_rx) = mpsc::channel(OUTPUT_BACKLOG);
        let handle = self.handle;

        // One task owns the channel and serves both directions.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    command = command_rx.recv() => match command {
                        Some(Command::Input(bytes)) => {
                            if channel.data_bytes(bytes).await.is_err() {
                                break;
                            }
                        }
                        Some(Command::Resize(cols, rows)) => {
                            let _ = channel.window_change(cols.into(), rows.into(), 0, 0).await;
                        }
                        // The `SshShell` was dropped.
                        None => break,
                    },
                    message = channel.wait() => match message {
                        Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                            if output_tx.send(data.to_vec()).await.is_err() {
                                break;
                            }
                        }
                        Some(ChannelMsg::Close) | None => break,
                        Some(_) => {}
                    },
                }
            }
            let _ = channel.close().await;
            let _ = handle
                .disconnect(Disconnect::ByApplication, "", "en")
                .await;
        });

        Ok((SshShell { commands: command_tx }, output_rx))
    }
}

enum Command {
    Input(Vec<u8>),
    Resize(u16, u16),
}

/// A remote shell. Dropping it closes the connection.
pub struct SshShell {
    commands: mpsc::Sender<Command>,
}

impl SshShell {
    pub async fn write(&self, bytes: Vec<u8>) {
        // A send error means the session already ended; the closed output
        // stream is how callers find out.
        let _ = self.commands.send(Command::Input(bytes)).await;
    }

    pub async fn resize(&self, cols: u16, rows: u16) {
        let _ = self.commands.send(Command::Resize(cols.max(1), rows.max(1))).await;
    }
}
