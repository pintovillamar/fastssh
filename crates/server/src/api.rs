//! JSON API for saved connections. Everything here needs an unlocked vault,
//! because saved secrets are read and written with the vault key.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, put},
};
use fastssh_core::{
    Secrets,
    ssh::{self, KeyError},
    store::{ConnectionDetails, SavedConnection},
    vault::VaultKey,
};
use serde::{Deserialize, Deserializer, Serialize};

use crate::AppState;
use crate::auth::Unlocked;
use crate::error::{ApiError, ApiResult};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/connections", get(list).post(create))
        .route("/connections/{id}", put(update).delete(remove))
        .route("/connections/{id}/host-key", delete(forget_host_key))
}

/// What the browser gets back. Secrets never leave the server; the interface
/// only learns which ones are saved.
#[derive(Serialize)]
struct ConnectionView {
    id: i64,
    #[serde(flatten)]
    details: ConnectionDetails,
    has_password: bool,
    has_private_key: bool,
    has_key_passphrase: bool,
}

impl ConnectionView {
    fn new(connection: SavedConnection, secrets: &Secrets) -> Self {
        Self {
            id: connection.id,
            details: connection.details,
            has_password: secrets.password.is_some(),
            has_private_key: secrets.private_key.is_some(),
            has_key_passphrase: secrets.key_passphrase.is_some(),
        }
    }
}

/// A change to one secret: leave the field out to keep what is saved, send
/// `null` or `""` to remove it, send text to replace it.
type Change = Option<Option<String>>;

fn change<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Change, D::Error> {
    Ok(Some(Option::deserialize(deserializer)?.filter(|text: &String| !text.is_empty())))
}

#[derive(Default, Deserialize)]
struct SecretChanges {
    #[serde(default, deserialize_with = "change")]
    password: Change,
    #[serde(default, deserialize_with = "change")]
    private_key: Change,
    #[serde(default, deserialize_with = "change")]
    key_passphrase: Change,
}

#[derive(Deserialize)]
struct ConnectionInput {
    #[serde(flatten)]
    details: ConnectionDetails,
    #[serde(default)]
    secrets: SecretChanges,
}

fn open(sealed: Option<&[u8]>, key: &VaultKey) -> ApiResult<Secrets> {
    Secrets::open(sealed, key).map_err(ApiError::internal)
}

/// Applies the requested changes and checks that a new private key is usable.
async fn apply(mut secrets: Secrets, changes: SecretChanges) -> ApiResult<Secrets> {
    if let Some(password) = changes.password {
        secrets.password = password;
    }
    if let Some(passphrase) = changes.key_passphrase {
        secrets.key_passphrase = passphrase;
    }
    if let Some(private_key) = changes.private_key {
        // Pasting often drops the final newline, which the key format requires.
        secrets.private_key = private_key.map(|text| format!("{}\n", text.trim()));
        if let Some(text) = &secrets.private_key {
            match ssh::decode_key(text.clone(), None).await {
                Ok(_) => {}
                Err(KeyError::NeedsPassphrase) => {
                    if let Some(passphrase) = &secrets.key_passphrase {
                        ssh::decode_key(text.clone(), Some(passphrase.clone()))
                            .await
                            .map_err(|_| ApiError::invalid("that passphrase does not unlock this key"))?;
                    }
                }
                Err(err) => return Err(ApiError::invalid(err.to_string())),
            }
        }
    }
    Ok(secrets)
}

async fn list(State(state): State<AppState>, session: Unlocked) -> ApiResult<Json<Vec<ConnectionView>>> {
    let connections = state.store.connections(session.user.id)?;
    let views = connections
        .into_iter()
        .map(|connection| {
            let secrets = open(connection.secrets.as_deref(), &session.key)?;
            Ok(ConnectionView::new(connection, &secrets))
        })
        .collect::<ApiResult<_>>()?;
    Ok(Json(views))
}

async fn create(
    State(state): State<AppState>,
    session: Unlocked,
    Json(input): Json<ConnectionInput>,
) -> ApiResult<(StatusCode, Json<ConnectionView>)> {
    let secrets = apply(Secrets::default(), input.secrets).await?;
    let saved = state
        .store
        .add_connection(session.user.id, input.details, secrets.seal(&session.key))?;
    Ok((StatusCode::CREATED, Json(ConnectionView::new(saved, &secrets))))
}

async fn update(
    State(state): State<AppState>,
    session: Unlocked,
    Path(id): Path<i64>,
    Json(input): Json<ConnectionInput>,
) -> ApiResult<Json<ConnectionView>> {
    let existing = state.store.connection(session.user.id, id)?;
    let secrets = open(existing.secrets.as_deref(), &session.key)?;
    let secrets = apply(secrets, input.secrets).await?;
    let saved = state
        .store
        .update_connection(session.user.id, id, input.details, secrets.seal(&session.key))?;
    Ok(Json(ConnectionView::new(saved, &secrets)))
}

async fn remove(State(state): State<AppState>, session: Unlocked, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    state.store.delete_connection(session.user.id, id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn forget_host_key(
    State(state): State<AppState>,
    session: Unlocked,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let connection = state.store.connection(session.user.id, id)?;
    state
        .store
        .forget_host(session.user.id, &connection.details.host, connection.details.port)?;
    Ok(StatusCode::NO_CONTENT)
}
