//! Web viewer for a datamine store: versions, tree browser, search, diff
//! and history. Every page has a CLI equivalent; this crate only renders
//! what `datamine-core` provides.

mod error;
mod pages;
mod state;
mod views;

use std::net::SocketAddr;
use std::path::Path;

use anyhow::{Context, Result};
use axum::Router;
use axum::http::header;
use axum::routing::get;

use crate::state::AppState;

const APP_CSS: &str = include_str!("../assets/app.css");
const APP_JS: &str = include_str!("../assets/app.js");

pub fn router(state: AppState) -> Router {
    use pages::{browse, records, versions};
    Router::new()
        .route("/", get(versions::list))
        .route("/v/{version}", get(versions::detail))
        .route("/v/{version}/browse", get(browse::root))
        .route("/v/{version}/browse/{*path}", get(browse::node))
        .route("/v/{version}/image/{*path}", get(browse::image))
        .route("/v/{version}/json/{*path}", get(browse::json))
        .route("/search", get(records::search))
        .route("/diff", get(records::diff))
        .route("/history", get(records::history))
        .route(
            "/assets/app.css",
            get(|| async { asset("text/css", APP_CSS) }),
        )
        .route(
            "/assets/app.js",
            get(|| async { asset("text/javascript", APP_JS) }),
        )
        .with_state(state)
}

fn asset(content_type: &'static str, body: &'static str) -> impl axum::response::IntoResponse {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
}

/// Serve the store at `addr` until Ctrl-C.
pub fn serve(store_root: &Path, addr: SocketAddr) -> Result<()> {
    let state = AppState::open(store_root)?;
    tokio::runtime::Runtime::new()?.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("binding {addr}"))?;
        println!("serving http://{addr}  (Ctrl-C to stop)");
        axum::serve(listener, router(state))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
        Ok(())
    })
}
