//! One websocket per terminal session.
//!
//! Binary frames carry raw terminal bytes in both directions. Text frames are
//! JSON control messages: [`ServerMsg`] to the browser, [`ClientMsg`] from it.
//!
//! An SSH session starts with a short conversation (host key, password or
//! passphrase) before the server sends `ready` and the byte stream begins.

use anyhow::{Result, bail};
use axum::{
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use fastssh_core::{
    LocalShell, Secrets, Shell,
    ssh::{self, KeyError},
    store::AuthKind,
};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::AppState;
use crate::auth::Unlocked;

const SECRET_ATTEMPTS: usize = 3;

#[derive(Deserialize)]
pub struct Params {
    #[serde(default = "default_cols")]
    cols: u16,
    #[serde(default = "default_rows")]
    rows: u16,
    /// Saved connection to open. Without it the session is a local shell.
    connection: Option<i64>,
}

fn default_cols() -> u16 {
    80
}

fn default_rows() -> u16 {
    24
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg<'a> {
    /// First contact with this server: ask whether to trust its key.
    HostKey {
        host: &'a str,
        port: u16,
        algorithm: &'a str,
        fingerprint: &'a str,
    },
    Prompt {
        kind: SecretKind,
        message: &'a str,
    },
    Error {
        message: &'a str,
    },
    Ready,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum SecretKind {
    Password,
    Passphrase,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Resize { cols: u16, rows: u16 },
    HostKeyReply { accept: bool },
    Secret { value: String },
    Cancel,
}

pub async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    account: Unlocked,
    headers: HeaderMap,
    Query(params): Query<Params>,
) -> Response {
    // `guard` already refuses a mismatched Origin. A websocket additionally
    // has to send one, so only browsers on our own page get this far.
    if !headers.contains_key(header::ORIGIN) {
        return (StatusCode::FORBIDDEN, "websocket without origin refused").into_response();
    }
    ws.on_upgrade(move |socket| session(socket, state, account, params))
}

/// The browser end of a session, plus the terminal size it last reported.
struct Browser {
    tx: SplitSink<WebSocket, Message>,
    rx: SplitStream<WebSocket>,
    cols: u16,
    rows: u16,
}

impl Browser {
    async fn send(&mut self, msg: ServerMsg<'_>) -> Result<()> {
        let json = serde_json::to_string(&msg)?;
        self.tx.send(Message::Text(json.into())).await?;
        Ok(())
    }

    /// Waits for the next answer during the opening conversation. Resizes are
    /// remembered rather than returned. `None` means the browser went away.
    async fn answer(&mut self) -> Option<ClientMsg> {
        while let Some(Ok(msg)) = self.rx.next().await {
            match msg {
                Message::Text(text) => match serde_json::from_str(&text) {
                    Ok(ClientMsg::Resize { cols, rows }) => (self.cols, self.rows) = (cols, rows),
                    Ok(other) => return Some(other),
                    Err(err) => tracing::warn!("ignoring control message: {err}"),
                },
                Message::Close(_) => return None,
                _ => {}
            }
        }
        None
    }

    async fn ask_secret(&mut self, kind: SecretKind, message: &str) -> Result<String> {
        self.send(ServerMsg::Prompt { kind, message }).await?;
        match self.answer().await {
            Some(ClientMsg::Secret { value }) => Ok(value),
            _ => bail!("cancelled"),
        }
    }
}

async fn session(socket: WebSocket, state: AppState, mut account: Unlocked, params: Params) {
    let (tx, rx) = socket.split();
    let mut browser = Browser {
        tx,
        rx,
        cols: params.cols,
        rows: params.rows,
    };

    let opened = match params.connection {
        None if state.config.local_shell_for(&account.user) => {
            LocalShell::spawn(browser.cols, browser.rows).map(|(shell, output)| (Shell::Local(shell), output))
        }
        None => Err(anyhow::anyhow!("the local shell is not available to this account")),
        Some(id) => open_ssh(&mut browser, &state, &account, id).await,
    };
    let (shell, output) = match opened {
        Ok(pair) => pair,
        Err(err) => {
            tracing::info!("session failed: {err:#}");
            let _ = browser.send(ServerMsg::Error { message: &format!("{err:#}") }).await;
            let _ = browser.tx.close().await;
            return;
        }
    };

    tracing::info!("session opened for {}", account.user.email);
    if browser.send(ServerMsg::Ready).await.is_ok() {
        tokio::select! {
            _ = pump(&mut browser, shell, output) => {}
            // Signing out ends every terminal the session had open.
            _ = account.ended() => {}
        }
    }
    let _ = browser.tx.close().await;
    tracing::info!("session closed");
}

async fn open_ssh(
    browser: &mut Browser,
    state: &AppState,
    account: &Unlocked,
    id: i64,
) -> Result<(Shell, mpsc::Receiver<Vec<u8>>)> {
    let connection = state.store.connection(account.user.id, id)?;
    let mut secrets = Secrets::open(connection.secrets.as_deref(), &account.key)?;

    // The connect future pauses inside the host key check until we answer its
    // question, so both have to be driven together.
    let (question_tx, mut questions) = mpsc::channel(1);
    let connect = ssh::connect(account.user.id, &connection, state.store.clone(), question_tx);
    tokio::pin!(connect);
    let mut client = loop {
        tokio::select! {
            result = &mut connect => break result?,
            Some(question) = questions.recv() => {
                browser.send(ServerMsg::HostKey {
                    host: &question.host,
                    port: question.port,
                    algorithm: &question.algorithm,
                    fingerprint: &question.fingerprint,
                }).await?;
                let accept = matches!(
                    browser.answer().await,
                    Some(ClientMsg::HostKeyReply { accept: true })
                );
                let _ = question.reply.send(accept);
            }
        }
    };

    let details = &connection.details;
    let who = format!("{}@{}", details.username, details.host);
    match details.auth {
        AuthKind::Password => {
            // A saved password gets the first try; if the server no longer
            // accepts it, fall back to asking.
            let mut accepted = match secrets.password.take() {
                Some(saved) => client.auth_password(saved).await?,
                None => false,
            };
            let mut message = format!("Password for {who}");
            for _ in 0..SECRET_ATTEMPTS {
                if accepted {
                    break;
                }
                let password = browser.ask_secret(SecretKind::Password, &message).await?;
                accepted = client.auth_password(password).await?;
                message = format!("Wrong password. Password for {who}");
            }
            if !accepted {
                bail!("authentication failed for {who}");
            }
        }
        AuthKind::Key => {
            let Some(text) = secrets.private_key.take() else {
                bail!("no private key is saved for this connection; edit it and add one");
            };
            let mut attempt = ssh::decode_key(text.clone(), secrets.key_passphrase.take()).await;
            let mut message = "Passphrase for this connection's private key".to_owned();
            for _ in 0..SECRET_ATTEMPTS {
                // An encrypted key with no passphrase, or with a wrong saved one.
                if !matches!(attempt, Err(KeyError::NeedsPassphrase | KeyError::Invalid(_))) {
                    break;
                }
                let passphrase = browser.ask_secret(SecretKind::Passphrase, &message).await?;
                attempt = ssh::decode_key(text.clone(), Some(passphrase)).await;
                message = "Wrong passphrase. Passphrase for this connection's private key".to_owned();
            }
            let key = attempt.map_err(|_| anyhow::anyhow!("could not unlock the private key"))?;
            if !client.auth_key(key).await? {
                bail!("{} did not accept this key for user {}", details.host, details.username);
            }
        }
    }

    let (shell, output) = client.open_shell(browser.cols, browser.rows).await?;
    Ok((Shell::Ssh(shell), output))
}

/// Shuttles bytes between the browser and the shell until either side ends.
async fn pump(browser: &mut Browser, shell: Shell, mut output: mpsc::Receiver<Vec<u8>>) {
    loop {
        tokio::select! {
            chunk = output.recv() => match chunk {
                Some(bytes) => {
                    if browser.tx.send(Message::Binary(bytes.into())).await.is_err() {
                        break;
                    }
                }
                // The shell exited.
                None => break,
            },
            msg = browser.rx.next() => match msg {
                Some(Ok(Message::Binary(bytes))) => shell.write(bytes.to_vec()).await,
                Some(Ok(Message::Text(text))) => match serde_json::from_str(&text) {
                    Ok(ClientMsg::Resize { cols, rows }) => {
                        if let Err(err) = shell.resize(cols, rows).await {
                            tracing::warn!("{err:#}");
                        }
                    }
                    Ok(_) => {}
                    Err(err) => tracing::warn!("ignoring control message: {err}"),
                },
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
        }
    }
}
