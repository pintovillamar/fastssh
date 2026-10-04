//! The HTTP API, driven in-process without opening a port.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use fastssh::Options;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Server {
    app: Router,
    // Deleted when the test ends.
    _data: tempfile::TempDir,
}

fn server(desktop: bool) -> Server {
    let data = tempfile::tempdir().unwrap();
    let app = fastssh::app(Options {
        data_dir: data.path().to_owned(),
        local_only: true,
        public_url: None,
        allow_signup: false,
        local_shell: true,
        google: None,
        desktop,
    })
    .unwrap();
    Server { app, _data: data }
}

struct Reply {
    status: StatusCode,
    body: Value,
    /// `name=value` of the session cookie, if one was set.
    cookie: Option<String>,
}

impl Server {
    async fn call(&self, method: &str, path: &str, cookie: Option<&str>, body: Option<Value>) -> Reply {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "127.0.0.1:7422");
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();

        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::to_owned);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            cookie,
        }
    }
}

#[tokio::test]
async fn server_sign_up_needs_an_email_and_guards_the_api() {
    let server = server(false);
    let session = server.call("GET", "/api/session", None, None).await;
    assert_eq!(session.body["state"], "setup");
    assert_eq!(session.body["desktop"], false);

    let no_email = json!({ "password": "correct horse" });
    assert_eq!(
        server.call("POST", "/api/signup", None, Some(no_email)).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        server.call("GET", "/api/connections", None, None).await.status,
        StatusCode::UNAUTHORIZED
    );

    let credentials = json!({ "email": "a@example.com", "password": "correct horse" });
    let created = server.call("POST", "/api/signup", None, Some(credentials)).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let cookie = created.cookie.expect("session cookie");

    let session = server.call("GET", "/api/session", Some(&cookie), None).await;
    assert_eq!(session.body["state"], "ready");
    assert_eq!(session.body["email"], "a@example.com");
    let list = server.call("GET", "/api/connections", Some(&cookie), None).await;
    assert_eq!(list.body, json!([]));
}

#[tokio::test]
async fn desktop_profile_needs_only_a_master_password() {
    let server = server(true);
    let session = server.call("GET", "/api/session", None, None).await;
    assert_eq!(session.body["state"], "setup");
    assert_eq!(session.body["desktop"], true);
    assert_eq!(session.body["signup"], false);

    let password = json!({ "password": "correct horse" });
    let created = server.call("POST", "/api/signup", None, Some(password.clone())).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let session = server
        .call("GET", "/api/session", created.cookie.as_deref(), None)
        .await;
    assert_eq!(session.body["state"], "ready");
    // Offered exactly where this build has a local shell (not on Windows yet).
    assert_eq!(session.body["local_shell"], fastssh_core::LocalShell::SUPPORTED);

    // There is exactly one profile.
    assert_eq!(
        server.call("POST", "/api/signup", None, Some(password.clone())).await.status,
        StatusCode::FORBIDDEN
    );

    let wrong = server
        .call("POST", "/api/login", None, Some(json!({ "password": "wrong password" })))
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.body["error"], "wrong password");

    let unlocked = server.call("POST", "/api/login", None, Some(password)).await;
    assert_eq!(unlocked.status, StatusCode::NO_CONTENT);
    let session = server
        .call("GET", "/api/session", unlocked.cookie.as_deref(), None)
        .await;
    assert_eq!(session.body["state"], "ready");
}

#[tokio::test]
async fn saved_secrets_are_reported_but_never_returned() {
    let server = server(true);
    let created = server
        .call("POST", "/api/signup", None, Some(json!({ "password": "correct horse" })))
        .await;
    let cookie = created.cookie.unwrap();

    let connection = json!({
        "name": "", "host": "example.com", "port": 22, "username": "root", "auth": "password",
        "secrets": { "password": "s3cret-value" }
    });
    let saved = server.call("POST", "/api/connections", Some(&cookie), Some(connection)).await;
    assert_eq!(saved.status, StatusCode::CREATED);
    assert_eq!(saved.body["name"], "root@example.com");
    assert_eq!(saved.body["has_password"], true);
    assert!(!saved.body.to_string().contains("s3cret-value"));

    let list = server.call("GET", "/api/connections", Some(&cookie), None).await;
    assert_eq!(list.body[0]["has_password"], true);
    assert!(!list.body.to_string().contains("s3cret-value"));
}

#[tokio::test]
async fn requests_from_other_sites_are_refused() {
    let server = server(false);
    let request = Request::builder()
        .method("POST")
        .uri("/api/logout")
        .header(header::HOST, "127.0.0.1:7422")
        .header(header::ORIGIN, "http://evil.example")
        .body(Body::empty())
        .unwrap();
    let response = server.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
