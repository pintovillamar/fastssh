//! One websocket per terminal session.
//!
//! Binary frames carry raw terminal bytes in both directions. Text frames are
//! JSON control messages from the browser (see [`Control`]).

use axum::{
    extract::{
        Query,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use fastssh_core::LocalShell;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Size {
    #[serde(default = "default_cols")]
    cols: u16,
    #[serde(default = "default_rows")]
    rows: u16,
}

fn default_cols() -> u16 {
    80
}

fn default_rows() -> u16 {
    24
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Control {
    Resize { cols: u16, rows: u16 },
}

pub async fn upgrade(ws: WebSocketUpgrade, headers: HeaderMap, Query(size): Query<Size>) -> Response {
    // Browsers let any website open a websocket to localhost, so without this
    // check a page you visit could quietly get a shell on your machine.
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "cross-origin websocket refused").into_response();
    }
    ws.on_upgrade(move |socket| session(socket, size))
}

fn same_origin(headers: &HeaderMap) -> bool {
    let get = |name| headers.get(name).and_then(|v| v.to_str().ok());
    let (Some(origin), Some(host)) = (get(header::ORIGIN), get(header::HOST)) else {
        return false;
    };
    origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .is_some_and(|origin_host| origin_host.eq_ignore_ascii_case(host))
}

async fn session(socket: WebSocket, size: Size) {
    let (shell, mut output) = match LocalShell::spawn(size.cols, size.rows) {
        Ok(pair) => pair,
        Err(err) => {
            tracing::error!("could not start shell: {err:#}");
            return;
        }
    };
    tracing::info!("session opened");

    let (mut to_browser, mut from_browser) = socket.split();
    loop {
        tokio::select! {
            chunk = output.recv() => match chunk {
                Some(bytes) => {
                    if to_browser.send(Message::Binary(bytes.into())).await.is_err() {
                        break;
                    }
                }
                // The shell exited.
                None => break,
            },
            msg = from_browser.next() => match msg {
                Some(Ok(Message::Binary(bytes))) => shell.write(bytes.to_vec()),
                Some(Ok(Message::Text(text))) => match serde_json::from_str(&text) {
                    Ok(Control::Resize { cols, rows }) => {
                        if let Err(err) = shell.resize(cols, rows) {
                            tracing::warn!("{err:#}");
                        }
                    }
                    Err(err) => tracing::warn!("ignoring control message: {err}"),
                },
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
        }
    }
    let _ = to_browser.close().await;
    tracing::info!("session closed");
}
