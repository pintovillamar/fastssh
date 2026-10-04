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
    LocalShell, Shell,
    ssh::{self, KeyError},
    store::AuthMethod,
};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::AppState;

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
    headers: HeaderMap,
    Query(params): Query<Params>,
) -> Response {
    // `guard` already refuses a mismatched Origin. A websocket additionally
    // has to send one, so only browsers on our own page get this far.
    if !headers.contains_key(header::ORIGIN) {
        return (StatusCode::FORBIDDEN, "websocket without origin refused").into_response();
    }
    ws.on_upgrade(move |socket| session(socket, state, params))
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

async fn session(socket: WebSocket, state: AppState, params: Params) {
    let (tx, rx) = socket.split();
    let mut browser = Browser {
        tx,
        rx,
        cols: params.cols,
        rows: params.rows,
    };

    let opened = match params.connection {
        None => LocalShell::spawn(browser.cols, browser.rows)
            .map(|(shell, output)| (Shell::Local(shell), output)),
        Some(id) => open_ssh(&mut browser, &state, id).await,
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

    tracing::info!("session opened");
    if browser.send(ServerMsg::Ready).await.is_ok() {
        pump(&mut browser, shell, output).await;
    }
    let _ = browser.tx.close().await;
    tracing::info!("session closed");
}

async fn open_ssh(
    browser: &mut Browser,
    state: &AppState,
    id: i64,
) -> Result<(Shell, mpsc::Receiver<Vec<u8>>)> {
    let connection = state.store.connection(id)?;

    // The connect future pauses inside the host key check until we answer its
    // question, so both have to be driven together.
    let (question_tx, mut questions) = mpsc::channel(1);
    let connect = ssh::connect(&connection, state.store.clone(), question_tx);
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
    match &details.auth {
        AuthMethod::Password => {
            let mut message = format!("Password for {who}");
            let mut accepted = false;
            for _ in 0..SECRET_ATTEMPTS {
                let password = browser.ask_secret(SecretKind::Password, &message).await?;
                if client.auth_password(password).await? {
                    accepted = true;
                    break;
                }
                message = format!("Wrong password. Password for {who}");
            }
            if !accepted {
                bail!("authentication failed for {who}");
            }
        }
        AuthMethod::KeyFile { path } => {
            let key = match ssh::load_key(path, None).await {
                Ok(key) => key,
                Err(KeyError::NeedsPassphrase) => {
                    let mut message = format!("Passphrase for {path}");
                    let mut unlocked = None;
                    for _ in 0..SECRET_ATTEMPTS {
                        let passphrase = browser.ask_secret(SecretKind::Passphrase, &message).await?;
                        if let Ok(key) = ssh::load_key(path, Some(passphrase)).await {
                            unlocked = Some(key);
                            break;
                        }
                        message = format!("Wrong passphrase. Passphrase for {path}");
                    }
                    match unlocked {
                        Some(key) => key,
                        None => bail!("could not unlock {path}"),
                    }
                }
                Err(err) => return Err(err.into()),
            };
            if !client.auth_key(key).await? {
                bail!("{} did not accept the key {path} for user {}", details.host, details.username);
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
