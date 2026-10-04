//! Accounts, sessions and vault unlocking.
//!
//! Being signed in and having the vault unlocked are separate things:
//!
//! - A **session** is a random token in a cookie; the database stores its
//!   hash. It survives server restarts.
//! - The **vault key** of a signed-in user is held only in this process's
//!   memory. A password login unlocks it straight away; after a Google login
//!   or a server restart the user is asked for the passphrase once.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    extract::{FromRequestParts, State},
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use fastssh_core::{
    store::{User, VaultRecord, normalize_email},
    vault::{self, Derived, VaultKey},
};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::AppState;
use crate::error::{ApiError, ApiResult};

const COOKIE: &str = "fastssh_session";
const SESSION_SECONDS: i64 = 30 * 24 * 60 * 60;
const MIN_PASSPHRASE_CHARS: usize = 8;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/session", get(session))
        .route("/signup", post(signup))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/unlock", post(unlock))
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

pub fn set_cookie(name: &str, value: &str, max_age: i64, secure: bool) -> HeaderValue {
    // HttpOnly keeps page scripts from reading it. SameSite=Lax keeps other
    // sites from sending it with their requests, while still working when
    // Google redirects the browser back to us.
    let secure = if secure { "; Secure" } else { "" };
    format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")
        .parse()
        .expect("cookie values are hex")
}

/// A signed-in user. Use as a handler argument to require a session.
pub struct Account {
    pub user: User,
    pub token_hash: [u8; 32],
}

impl Account {
    fn find(headers: &HeaderMap, state: &AppState) -> ApiResult<Option<Self>> {
        let Some(token) = cookie(headers, COOKIE) else {
            return Ok(None);
        };
        let token_hash = vault::sha256(token.as_bytes());
        Ok(state
            .store
            .session_user(&token_hash, now())?
            .map(|user| Self { user, token_hash }))
    }
}

impl FromRequestParts<AppState> for Account {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        Self::find(&parts.headers, state)?.ok_or_else(ApiError::signed_out)
    }
}

/// A signed-in user whose vault key is in memory. Use as a handler argument
/// for anything that reads or writes secrets.
pub struct Unlocked {
    pub user: User,
    pub key: VaultKey,
    /// Closes when the session signs out; see [`Unlocked::ended`].
    alive: watch::Receiver<()>,
}

impl Unlocked {
    /// Completes when this session signs out, so long-lived work started
    /// under it (an open terminal) can stop.
    pub async fn ended(&mut self) {
        while self.alive.changed().await.is_ok() {}
    }
}

impl FromRequestParts<AppState> for Unlocked {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        let account = Account::from_request_parts(parts, state).await?;
        let (key, alive) = state.keys.get(&account.token_hash).ok_or_else(ApiError::locked)?;
        Ok(Self {
            user: account.user,
            key,
            alive,
        })
    }
}

/// Vault keys of unlocked sessions, by session token hash. Memory only.
///
/// Each entry also owns a channel that closes when the entry is removed,
/// which is how open terminals learn that their session signed out.
#[derive(Default)]
pub struct Keys(Mutex<UnlockedSessions>);

type UnlockedSessions = HashMap<[u8; 32], (VaultKey, watch::Sender<()>)>;

impl Keys {
    fn lock(&self) -> std::sync::MutexGuard<'_, UnlockedSessions> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn get(&self, token_hash: &[u8; 32]) -> Option<(VaultKey, watch::Receiver<()>)> {
        self.lock()
            .get(token_hash)
            .map(|(key, alive)| (key.clone(), alive.subscribe()))
    }

    fn is_unlocked(&self, token_hash: &[u8; 32]) -> bool {
        self.lock().contains_key(token_hash)
    }

    fn insert(&self, token_hash: [u8; 32], key: VaultKey) {
        self.lock().insert(token_hash, (key, watch::channel(()).0));
    }

    fn remove(&self, token_hash: &[u8; 32]) {
        self.lock().remove(token_hash);
    }
}

/// Slows down passphrase guessing: after a few failures for one email,
/// further attempts are refused for a while. The price is that someone who
/// knows your email can lock you out for those minutes.
#[derive(Default)]
pub struct Limiter(Mutex<HashMap<String, (u32, Instant)>>);

impl Limiter {
    const MAX_FAILURES: u32 = 5;
    const WINDOW: Duration = Duration::from_secs(5 * 60);

    fn check(&self, email: &str) -> ApiResult<()> {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match map.get(email) {
            Some((failures, since)) if *failures >= Self::MAX_FAILURES && since.elapsed() < Self::WINDOW => {
                Err(ApiError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "too many failed attempts; try again in a few minutes",
                ))
            }
            _ => Ok(()),
        }
    }

    fn failed(&self, email: &str) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 10_000 {
            map.retain(|_, (_, since)| since.elapsed() < Self::WINDOW);
        }
        let entry = map.entry(email.to_owned()).or_insert((0, Instant::now()));
        if entry.1.elapsed() >= Self::WINDOW {
            *entry = (0, Instant::now());
        }
        entry.0 += 1;
    }

    fn succeeded(&self, email: &str) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(email);
    }
}

/// Argon2 is slow on purpose, so it runs off the async threads.
async fn derive(passphrase: String, salt: Vec<u8>) -> ApiResult<Derived> {
    tokio::task::spawn_blocking(move || Derived::from_passphrase(&passphrase, &salt))
        .await
        .map_err(ApiError::internal)
}

fn check_passphrase(passphrase: &str) -> ApiResult<()> {
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(ApiError::invalid(format!(
            "use at least {MIN_PASSPHRASE_CHARS} characters"
        )));
    }
    Ok(())
}

/// Starts a session for `user_id` and returns the cookie that carries it.
/// With `key` the session starts unlocked.
pub fn start_session(state: &AppState, user_id: i64, key: Option<VaultKey>) -> ApiResult<HeaderValue> {
    let token = vault::new_token();
    let token_hash = vault::sha256(token.as_bytes());
    let now = now();
    state
        .store
        .create_session(&token_hash, user_id, now, now + SESSION_SECONDS)?;
    if let Some(key) = key {
        state.keys.insert(token_hash, key);
    }
    Ok(set_cookie(COOKIE, &token, SESSION_SECONDS, state.config.secure_cookies))
}

#[derive(Serialize)]
struct SessionInfo {
    /// `setup`: no accounts exist yet. `signed_out`. `new_vault`: signed in
    /// but has never set a vault passphrase. `locked`. `ready`.
    state: &'static str,
    email: Option<String>,
    /// Whether the vault passphrase is the account's login password.
    has_password: bool,
    google: bool,
    signup: bool,
    local_shell: bool,
    /// The desktop app: one local profile, identified by nothing but its
    /// master password.
    desktop: bool,
}

async fn session(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<SessionInfo>> {
    let account = Account::find(&headers, &state)?;
    let phase = match &account {
        None if !state.store.has_users()? => "setup",
        None => "signed_out",
        Some(account) if account.user.vault.is_none() => "new_vault",
        Some(account) if !state.keys.is_unlocked(&account.token_hash) => "locked",
        Some(_) => "ready",
    };
    let user = account.map(|account| account.user);
    Ok(Json(SessionInfo {
        state: phase,
        has_password: user
            .as_ref()
            .and_then(|u| u.vault.as_ref())
            .is_some_and(|v| v.verifier.is_some()),
        local_shell: user.as_ref().is_some_and(|u| state.config.local_shell_for(u)),
        email: user.map(|u| u.email),
        google: state.config.google.is_some(),
        signup: state.config.allow_signup,
        desktop: state.config.desktop,
    }))
}

#[derive(Deserialize)]
struct Credentials {
    #[serde(default)]
    email: String,
    password: String,
}

/// The desktop app's single profile still needs an account row; this is the
/// address it is stored under. Nobody types it.
const DESKTOP_EMAIL: &str = "desktop@fastssh.local";

impl Credentials {
    fn email(&self, state: &AppState) -> &str {
        if state.config.desktop { DESKTOP_EMAIL } else { &self.email }
    }
}

async fn signup(State(state): State<AppState>, Json(body): Json<Credentials>) -> ApiResult<Response> {
    // Checked again inside `create_user`; this just avoids the slow key
    // derivation for requests that cannot succeed.
    if state.store.has_users()? && !state.config.allow_signup {
        return Err(fastssh_core::store::StoreError::SignupClosed.into());
    }
    let email = body.email(&state).to_owned();
    normalize_email(&email)?;
    check_passphrase(&body.password)?;

    let salt = vault::new_salt().to_vec();
    let derived = derive(body.password, salt.clone()).await?;
    let key = VaultKey::generate();
    let record = VaultRecord {
        salt,
        verifier: Some(derived.verifier().to_vec()),
        wrapped_key: key.wrap(&derived),
    };
    let user = state
        .store
        .create_user(&email, Some(record), state.config.allow_signup, now())?;
    tracing::info!("account created: {}", user.email);

    let cookie = start_session(&state, user.id, Some(key))?;
    Ok((StatusCode::CREATED, [(header::SET_COOKIE, cookie)]).into_response())
}

async fn login(State(state): State<AppState>, Json(body): Json<Credentials>) -> ApiResult<Response> {
    let email = body.email(&state).trim().to_lowercase();
    state.limiter.check(&email)?;

    let user = state.store.user_by_email(&email)?;
    let record = user
        .as_ref()
        .and_then(|u| u.vault.as_ref())
        .filter(|v| v.verifier.is_some());
    // Unknown accounts still pay for a derivation, so the response time does
    // not reveal which emails are registered.
    let salt = record.map_or_else(|| vec![0; vault::SALT_LEN], |v| v.salt.clone());
    let derived = derive(body.password, salt).await?;

    let unlocked = record.and_then(|record| {
        let verifier = record.verifier.as_deref()?;
        derived
            .verifier_matches(verifier)
            .then(|| VaultKey::unwrap(&record.wrapped_key, &derived))?
    });
    let (Some(user), Some(key)) = (user, unlocked) else {
        state.limiter.failed(&email);
        let message = if state.config.desktop { "wrong password" } else { "wrong email or password" };
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, message));
    };
    state.limiter.succeeded(&email);

    let cookie = start_session(&state, user.id, Some(key))?;
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response())
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(account) = Account::find(&headers, &state)? {
        state.keys.remove(&account.token_hash);
        state.store.delete_session(&account.token_hash)?;
    }
    let expired = set_cookie(COOKIE, "", 0, state.config.secure_cookies);
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, expired)]).into_response())
}

#[derive(Deserialize)]
struct Unlock {
    passphrase: String,
}

/// Unlocks the vault of the current session, or creates the vault if the
/// account (one that signs in with Google) has none yet.
async fn unlock(State(state): State<AppState>, account: Account, Json(body): Json<Unlock>) -> ApiResult<StatusCode> {
    let email = &account.user.email;
    let key = match &account.user.vault {
        Some(record) => {
            state.limiter.check(email)?;
            let derived = derive(body.passphrase, record.salt.clone()).await?;
            match VaultKey::unwrap(&record.wrapped_key, &derived) {
                Some(key) => {
                    state.limiter.succeeded(email);
                    key
                }
                None => {
                    state.limiter.failed(email);
                    return Err(ApiError::new(StatusCode::UNAUTHORIZED, "wrong passphrase"));
                }
            }
        }
        None => {
            check_passphrase(&body.passphrase)?;
            let salt = vault::new_salt().to_vec();
            let derived = derive(body.passphrase, salt.clone()).await?;
            let key = VaultKey::generate();
            state.store.create_vault(
                account.user.id,
                &VaultRecord {
                    salt,
                    verifier: None,
                    wrapped_key: key.wrap(&derived),
                },
            )?;
            key
        }
    };
    state.keys.insert(account.token_hash, key);
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_cookie_among_several() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, "a=1; fastssh_session=abc; b=2".parse().unwrap());
        assert_eq!(cookie(&headers, "fastssh_session"), Some("abc"));
        assert_eq!(cookie(&headers, "session"), None);
    }

    #[test]
    fn limiter_blocks_after_repeated_failures_until_success() {
        let limiter = Limiter::default();
        for _ in 0..Limiter::MAX_FAILURES {
            assert!(limiter.check("a@b.c").is_ok());
            limiter.failed("a@b.c");
        }
        assert!(limiter.check("a@b.c").is_err());
        assert!(limiter.check("other@b.c").is_ok());
        limiter.succeeded("a@b.c");
        assert!(limiter.check("a@b.c").is_ok());
    }
}
