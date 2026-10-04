//! Web viewer for a datamine store: a game database (skills, ...), tools
//! like the skill builder, patch diffs and a raw data browser. Every page
//! has a CLI equivalent; this crate only renders what `datamine-core`
//! provides.

mod assets;
mod error;
mod pages;
mod site;
mod state;
mod views;

use std::net::SocketAddr;
use std::path::Path;

use anyhow::{Context, Result};
use axum::Router;
use axum::routing::get;
use tower_http::compression::CompressionLayer;

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    use pages::{browse, builder, home, records, skills, versions};
    Router::new()
        .route("/", get(home::home))
        .route("/skills", get(skills::index))
        .route("/skills/job/{job}", get(skills::job))
        .route("/skills/{id}", get(skills::detail))
        .route("/tools/skill-builder", get(builder::builder))
        .route("/versions", get(versions::list))
        .route("/v/{version}", get(versions::detail))
        .route("/v/{version}/browse", get(browse::root))
        .route("/v/{version}/browse/{*path}", get(browse::node))
        .route("/v/{version}/image/{*path}", get(browse::image))
        .route("/v/{version}/json/{*path}", get(browse::json))
        .route("/search", get(records::search))
        .route("/diff", get(records::diff))
        .route("/history", get(records::history))
        .route("/assets/{name}", get(assets::serve))
        // Pages are repetitive HTML; gzip/brotli shrink them ~10x.
        .layer(CompressionLayer::new())
        .with_state(state)
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
