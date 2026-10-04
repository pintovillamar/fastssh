//! The error every API handler returns: a status and a message for the user.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use fastssh_core::store::StoreError;
use serde_json::json;

pub struct ApiError {
    status: StatusCode,
    message: String,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, message)
    }

    pub fn signed_out() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "sign in to continue")
    }

    /// Signed in, but the vault key is not in memory (for example after a
    /// server restart). The interface answers this by asking for the passphrase.
    pub fn locked() -> Self {
        Self::new(StatusCode::LOCKED, "unlock your vault to continue")
    }

    /// For failures the user can do nothing about. The detail goes to the log.
    pub fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!("{err}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "something went wrong on the server")
    }
}

impl From<StoreError> for ApiError {
    fn from(err: StoreError) -> Self {
        let status = match &err {
            StoreError::NotFound => StatusCode::NOT_FOUND,
            StoreError::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            StoreError::EmailTaken => StatusCode::CONFLICT,
            StoreError::SignupClosed => StatusCode::FORBIDDEN,
            StoreError::Db(db) => return Self::internal(db),
        };
        Self::new(status, err.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}
