//! Static assets compiled into the binary.
//!
//! Pages link to `/assets/<name>?h=<content hash>`, so a new build changes
//! the URL and browsers can cache each version forever.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::LazyLock;

use axum::extract::{Path, Query};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

struct Asset {
    name: &'static str,
    content_type: &'static str,
    body: &'static str,
}

const ASSETS: &[Asset] = &[
    Asset {
        name: "app.css",
        content_type: "text/css",
        body: include_str!("../assets/app.css"),
    },
    Asset {
        name: "app.js",
        content_type: "text/javascript",
        body: include_str!("../assets/app.js"),
    },
    Asset {
        name: "builder.js",
        content_type: "text/javascript",
        body: include_str!("../assets/builder.js"),
    },
];

static HASHES: LazyLock<HashMap<&'static str, String>> = LazyLock::new(|| {
    ASSETS
        .iter()
        .map(|a| {
            let mut h = DefaultHasher::new();
            a.body.hash(&mut h);
            (a.name, format!("{:x}", h.finish()))
        })
        .collect()
});

/// Cache-busting URL of an asset.
pub fn url(name: &str) -> String {
    match HASHES.get(name) {
        Some(h) => format!("/assets/{name}?h={h}"),
        None => format!("/assets/{name}"),
    }
}

#[derive(Deserialize)]
pub struct HashParam {
    h: Option<String>,
}

pub async fn serve(Path(name): Path<String>, Query(q): Query<HashParam>) -> Response {
    let Some(asset) = ASSETS.iter().find(|a| a.name == name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // Only hashed URLs are immutable; a bare URL must revalidate.
    let cache = if q.h.as_deref() == HASHES.get(asset.name).map(String::as_str) {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, asset.content_type),
            (header::CACHE_CONTROL, cache),
        ],
        asset.body,
    )
        .into_response()
}
