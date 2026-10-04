//! JSON API for saved connections.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, put},
};
use fastssh_core::store::{ConnectionDetails, SavedConnection, StoreError};
use serde_json::json;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/connections", get(list).post(create))
        .route("/connections/{id}", put(update).delete(remove))
        .route("/connections/{id}/host-key", delete(forget_host_key))
}

struct ApiError(StoreError);

impl From<StoreError> for ApiError {
    fn from(err: StoreError) -> Self {
        Self(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            StoreError::NotFound => StatusCode::NOT_FOUND,
            StoreError::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            StoreError::Db(err) => {
                tracing::error!("{err}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, Json(json!({ "error": self.0.to_string() }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

async fn list(State(state): State<AppState>) -> ApiResult<Json<Vec<SavedConnection>>> {
    Ok(Json(state.store.connections()?))
}

async fn create(
    State(state): State<AppState>,
    Json(details): Json<ConnectionDetails>,
) -> ApiResult<(StatusCode, Json<SavedConnection>)> {
    Ok((StatusCode::CREATED, Json(state.store.add_connection(details)?)))
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(details): Json<ConnectionDetails>,
) -> ApiResult<Json<SavedConnection>> {
    Ok(Json(state.store.update_connection(id, details)?))
}

async fn remove(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    state.store.delete_connection(id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn forget_host_key(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let connection = state.store.connection(id)?;
    state
        .store
        .forget_host(&connection.details.host, connection.details.port)?;
    Ok(StatusCode::NO_CONTENT)
}
