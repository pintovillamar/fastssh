//! Request checks that apply before any handler runs.
//!
//! - Requests whose `Origin` is a different site are refused. Together with
//!   the SameSite session cookie this stops other websites from acting as a
//!   signed-in user (cross-site request forgery), websockets included.
//! - When listening on localhost, requests whose `Host` is neither a
//!   localhost name nor the configured public address are refused. That
//!   stops DNS rebinding, where a hostile domain is pointed at 127.0.0.1.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

#[derive(Clone)]
pub struct Policy {
    pub local_only: bool,
    /// `host[:port]` of `--public-url`, as a browser would send it in `Host`.
    pub public_host: Option<String>,
}

pub async fn check(State(policy): State<Policy>, request: Request, next: Next) -> Response {
    if allowed(&policy, request.headers()) {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "request refused").into_response()
    }
}

fn allowed(policy: &Policy, headers: &HeaderMap) -> bool {
    let get = |name| headers.get(name).and_then(|v| v.to_str().ok());
    let Some(host) = get(header::HOST) else {
        return false;
    };
    let is_public = |name: &str| {
        policy
            .public_host
            .as_deref()
            .is_some_and(|public| public.eq_ignore_ascii_case(name))
    };
    if policy.local_only && !is_local_name(host) && !is_public(host) {
        return false;
    }
    match get(header::ORIGIN) {
        None => true,
        Some(origin) => origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"))
            // A reverse proxy may rewrite Host, so the public address counts too.
            .is_some_and(|origin_host| origin_host.eq_ignore_ascii_case(host) || is_public(origin_host)),
    }
}

/// Whether a `Host` header value (with optional port) names this machine.
pub fn is_local_name(host: &str) -> bool {
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

    fn local() -> Policy {
        Policy {
            local_only: true,
            public_host: None,
        }
    }

    #[test]
    fn accepts_same_origin_local_requests() {
        assert!(allowed(&local(), &headers("127.0.0.1:7422", None)));
        assert!(allowed(&local(), &headers("localhost:7422", Some("http://localhost:7422"))));
        assert!(allowed(&local(), &headers("[::1]:7422", Some("http://[::1]:7422"))));
    }

    #[test]
    fn refuses_other_sites() {
        assert!(!allowed(&local(), &headers("127.0.0.1:7422", Some("http://evil.example"))));
        assert!(!allowed(&local(), &headers("127.0.0.1:7422", Some("null"))));
    }

    #[test]
    fn refuses_rebound_hostnames_when_local_only() {
        let rebound = headers("evil.example:7422", Some("http://evil.example:7422"));
        assert!(!allowed(&local(), &rebound));
        let open = Policy {
            local_only: false,
            public_host: None,
        };
        assert!(allowed(&open, &rebound));
    }

    #[test]
    fn accepts_the_public_address_behind_a_reverse_proxy() {
        let proxied = Policy {
            local_only: true,
            public_host: Some("ssh.example.com".into()),
        };
        assert!(allowed(&proxied, &headers("ssh.example.com", Some("https://ssh.example.com"))));
        // Proxy that rewrites Host to the upstream address.
        assert!(allowed(&proxied, &headers("127.0.0.1:7422", Some("https://ssh.example.com"))));
        assert!(!allowed(&proxied, &headers("ssh.example.com", Some("https://evil.example"))));
        assert!(!allowed(&proxied, &headers("evil.example", None)));
    }
}
