//! "Sign in with Google" (OAuth 2 authorization code flow with PKCE).
//!
//! Google only tells us who the person is. It cannot unlock the vault, so a
//! Google sign-in always lands on the passphrase prompt.
//!
//! 1. `/api/auth/google` sends the browser to Google with a random `state`.
//! 2. Google sends it back to `/api/auth/google/callback` with a `code`.
//! 3. We swap the code for an access token (server to server) and ask Google
//!    for the verified email address behind it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::{
    Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use base64::{Engine, prelude::BASE64_URL_SAFE_NO_PAD};
use fastssh_core::{store::StoreError, vault};
use serde::Deserialize;

use crate::AppState;
use crate::auth::{self, cookie, set_cookie};

const STATE_COOKIE: &str = "fastssh_oauth";
const FLOW_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub struct Google {
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    http: reqwest::Client,
    /// Sign-ins in progress: `state` -> (PKCE verifier, started).
    pending: Mutex<HashMap<String, (String, Instant)>>,
}

impl Google {
    pub fn new(client_id: String, client_secret: String) -> Self {
        // The endpoint overrides exist so tests can stand in for Google.
        let endpoint = |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.to_owned());
        Self {
            client_id,
            client_secret,
            auth_url: endpoint("FASTSSH_GOOGLE_AUTH_URL", "https://accounts.google.com/o/oauth2/v2/auth"),
            token_url: endpoint("FASTSSH_GOOGLE_TOKEN_URL", "https://oauth2.googleapis.com/token"),
            userinfo_url: endpoint(
                "FASTSSH_GOOGLE_USERINFO_URL",
                "https://openidconnect.googleapis.com/v1/userinfo",
            ),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("http client"),
            pending: Mutex::default(),
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/google", get(start))
        .route("/auth/google/callback", get(callback))
}

fn redirect_uri(state: &AppState, headers: &HeaderMap) -> String {
    format!("{}/api/auth/google/callback", state.config.base_url(headers))
}

async fn start(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(google) = &state.config.google else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let flow = vault::new_token();
    let verifier = vault::new_token();
    let challenge = BASE64_URL_SAFE_NO_PAD.encode(vault::sha256(verifier.as_bytes()));
    {
        let mut pending = google.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.retain(|_, (_, started)| started.elapsed() < FLOW_TIMEOUT);
        pending.insert(flow.clone(), (verifier, Instant::now()));
    }

    let mut url = url::Url::parse(&google.auth_url).expect("auth url checked at startup");
    url.query_pairs_mut()
        .append_pair("client_id", &google.client_id)
        .append_pair("redirect_uri", &redirect_uri(&state, &headers))
        .append_pair("response_type", "code")
        .append_pair("scope", "openid email")
        .append_pair("state", &flow)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");

    // The cookie ties the flow to this browser, so nobody can finish a
    // sign-in that somebody else started.
    let cookie = set_cookie(
        STATE_COOKIE,
        &flow,
        FLOW_TIMEOUT.as_secs() as i64,
        state.config.secure_cookies,
    );
    ([(header::SET_COOKIE, cookie)], Redirect::to(url.as_str())).into_response()
}

#[derive(Deserialize)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
}

#[derive(Deserialize)]
struct Token {
    access_token: String,
}

#[derive(Deserialize)]
struct UserInfo {
    email: Option<String>,
    #[serde(default)]
    email_verified: bool,
}

async fn callback(State(state): State<AppState>, headers: HeaderMap, Query(query): Query<Callback>) -> Response {
    let clear = set_cookie(STATE_COOKIE, "", 0, state.config.secure_cookies);
    match finish(&state, &headers, query).await {
        Ok(session) => (
            [(header::SET_COOKIE, clear), (header::SET_COOKIE, session)],
            Redirect::to("/"),
        )
            .into_response(),
        Err(message) => {
            tracing::info!("google sign-in failed: {message}");
            let query: String = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("error", &message)
                .finish();
            ([(header::SET_COOKIE, clear)], Redirect::to(&format!("/?{query}"))).into_response()
        }
    }
}

/// Returns the session cookie, or a message to show on the sign-in screen.
async fn finish(state: &AppState, headers: &HeaderMap, query: Callback) -> Result<header::HeaderValue, String> {
    const FAILED: &str = "Google sign-in failed. Please try again.";
    let google = state.config.google.as_ref().ok_or(FAILED)?;
    let (Some(code), Some(flow)) = (query.code, query.state) else {
        return Err("Google sign-in was cancelled.".into());
    };
    if cookie(headers, STATE_COOKIE) != Some(flow.as_str()) {
        return Err(FAILED.into());
    }
    let verifier = google
        .pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&flow)
        .filter(|(_, started)| started.elapsed() < FLOW_TIMEOUT)
        .map(|(verifier, _)| verifier)
        .ok_or("Google sign-in took too long. Please try again.")?;

    let log = |err: reqwest::Error| {
        tracing::warn!("google: {err}");
        FAILED.to_owned()
    };
    let token: Token = google
        .http
        .post(&google.token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &google.client_id),
            ("client_secret", &google.client_secret),
            ("redirect_uri", &redirect_uri(state, headers)),
            ("code_verifier", &verifier),
        ])
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(log)?
        .json()
        .await
        .map_err(log)?;
    let info: UserInfo = google
        .http
        .get(&google.userinfo_url)
        .bearer_auth(&token.access_token)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(log)?
        .json()
        .await
        .map_err(log)?;

    let email = match info.email {
        Some(email) if info.email_verified => email,
        _ => return Err("Your Google account has no verified email address.".into()),
    };

    let internal = |err: StoreError| {
        tracing::error!("{err}");
        FAILED.to_owned()
    };
    let user = match state.store.user_by_email(&email).map_err(internal)? {
        Some(user) => user,
        // Google accounts start without a vault; the passphrase prompt creates it.
        None => match state
            .store
            .create_user(&email, None, state.config.allow_signup, auth::now())
        {
            Ok(user) => {
                tracing::info!("account created via Google: {}", user.email);
                user
            }
            Err(StoreError::SignupClosed) => {
                return Err(format!("There is no account for {email} on this server."));
            }
            Err(err) => return Err(internal(err)),
        },
    };
    auth::start_session(state, user.id, None).map_err(|_| FAILED.to_owned())
}
