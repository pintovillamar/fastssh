//! The web interface, compiled into the binary.
//!
//! Release builds embed `web/dist`, so deploying is copying one file. Debug
//! builds read the folder from disk on every request instead.

use axum::{
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../web/dist"]
struct Web;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    // Unknown paths get the app shell so client-side routes survive a reload.
    let Some((name, file)) = Web::get(path)
        .map(|f| (path, f))
        .or_else(|| Web::get("index.html").map(|f| ("index.html", f)))
    else {
        return (
            StatusCode::NOT_FOUND,
            "web interface not built: run `npm run build` in web/",
        )
            .into_response();
    };

    let mime = mime_guess::from_path(name).first_or_octet_stream();
    // Files under assets/ have a content hash in their name, so browsers may
    // keep them forever. The page itself must always be asked for again, or
    // an update would never be picked up.
    let caching = if name.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, mime.as_ref()),
            (header::CACHE_CONTROL, caching),
        ],
        file.data,
    )
        .into_response()
}
