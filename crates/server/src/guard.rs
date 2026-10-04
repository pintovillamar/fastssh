//! Request checks that stand in for authentication until logins exist.
//!
//! A browser will happily let any website talk to a server on localhost, so
//! two things are refused here:
//!
//! - requests whose `Origin` is a different site (cross-site requests and
//!   websockets), and
//! - when listening on localhost, requests whose `Host` is not a localhost
//!   name. That stops DNS rebinding, where a hostile domain is pointed at
//!   127.0.0.1 so its pages look same-origin to us.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

#[derive(Clone, Copy)]
pub struct Policy {
    pub local_only: bool,
}

pub async fn check(State(policy): State<Policy>, request: Request, next: Next) -> Response {
    if allowed(policy, request.headers()) {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "request refused").into_response()
    }
}

fn allowed(policy: Policy, headers: &HeaderMap) -> bool {
    let get = |name| headers.get(name).and_then(|v| v.to_str().ok());
    let Some(host) = get(header::HOST) else {
        return false;
    };
    if policy.local_only && !is_local_name(host) {
        return false;
    }
    match get(header::ORIGIN) {
        None => true,
        Some(origin) => origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"))
            .is_some_and(|origin_host| origin_host.eq_ignore_ascii_case(host)),
    }
}

/// Whether a `Host` header value (with optional port) names this machine.
fn is_local_name(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        // IPv6 literal: "[::1]:7422"
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1" || name == "::1"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(host: &str, origin: Option<&str>) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(header::HOST, host.parse().unwrap());
        if let Some(origin) = origin {
            map.insert(header::ORIGIN, origin.parse().unwrap());
        }
        map
    }

    const LOCAL: Policy = Policy { local_only: true };

    #[test]
    fn accepts_same_origin_local_requests() {
        assert!(allowed(LOCAL, &headers("127.0.0.1:7422", None)));
        assert!(allowed(LOCAL, &headers("localhost:7422", Some("http://localhost:7422"))));
        assert!(allowed(LOCAL, &headers("[::1]:7422", Some("http://[::1]:7422"))));
    }

    #[test]
    fn refuses_other_sites() {
        assert!(!allowed(LOCAL, &headers("127.0.0.1:7422", Some("http://evil.example"))));
        assert!(!allowed(LOCAL, &headers("127.0.0.1:7422", Some("null"))));
    }

    #[test]
    fn refuses_rebound_hostnames_when_local_only() {
        let rebound = headers("evil.example:7422", Some("http://evil.example:7422"));
        assert!(!allowed(LOCAL, &rebound));
        assert!(allowed(Policy { local_only: false }, &rebound));
    }
}
