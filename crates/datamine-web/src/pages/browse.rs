//! Tree browser over a version's snapshot.
//! CLI equivalents: `datamine ls`, `datamine show`, `datamine export-image`.

use std::io::Cursor;

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Redirect, Response};
use datamine_core::wz::{self, JsonOptions};
use maud::html;
use serde::Deserialize;
use serde_json::Value;

use crate::error::AppError;
use crate::site::Site;
use crate::state::AppState;
use crate::views::{self, Nav, badge, encode_path};

const PAGE_SIZE: usize = 200;

#[derive(Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    page: usize,
    /// Set by the header's version switcher: jump to the same path there.
    v: Option<String>,
}

struct ChildRow {
    name: String,
    kind: &'static str,
    preview: Option<String>,
    has_children: bool,
}

struct NodeView {
    site: Site,
    label: String,
    kind: &'static str,
    value: Option<Value>,
    children: Vec<ChildRow>,
    total_children: usize,
}

pub async fn root(
    state: State<AppState>,
    Path(version): Path<String>,
    query: Query<PageQuery>,
) -> Result<Response, AppError> {
    node(state, Path((version, String::new())), query).await
}

pub async fn node(
    State(state): State<AppState>,
    Path((version, path)): Path<(String, String)>,
    Query(q): Query<PageQuery>,
) -> Result<Response, AppError> {
    let path = path.trim_matches('/').to_owned();
    if let Some(target) = q.v.as_deref().filter(|t| *t != version && !t.is_empty()) {
        let to = format!("/v/{}/browse/{}", encode_path(target), encode_path(&path));
        return Ok(Redirect::to(&to).into_response());
    }
    let lookup = path.clone();
    let here = format!("/v/{}/browse/{}", encode_path(&version), encode_path(&path));
    let view = state
        .run(move |ctx| {
            let site = Site::load(ctx, Some(&version), &here)?;
            let v = site.version.clone();
            let tree = ctx.tree(&v)?;
            let node = tree.get(&lookup).map_err(AppError::NotFound)?;

            let mut children = wz::children(&node);
            children.sort_by(|a, b| wz::natural_cmp(&a.0, &b.0));
            let total_children = children.len();
            let rows = children
                .into_iter()
                .skip(q.page * PAGE_SIZE)
                .take(PAGE_SIZE)
                .map(|(name, child)| {
                    let is_value = wz::is_value(&child);
                    ChildRow {
                        kind: wz::type_name(&child),
                        preview: if is_value {
                            Some(views::value_text(&wz::node_to_json(
                                &child,
                                JsonOptions::default(),
                            )))
                        } else {
                            wz::display_name(&child)
                        },
                        has_children: !is_value,
                        name,
                    }
                })
                .collect();

            Ok(NodeView {
                site,
                label: v.label,
                kind: wz::type_name(&node),
                value: wz::is_value(&node).then(|| wz::node_to_json(&node, JsonOptions::default())),
                children: rows,
                total_children,
            })
        })
        .await?;

    let base = format!("/v/{}", encode_path(&view.label));
    let title = path
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(&view.label);
    let mut crumbs = vec![("Raw data".to_owned(), format!("{base}/browse"))];
    let mut acc = String::new();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        acc = if acc.is_empty() {
            seg.to_owned()
        } else {
            format!("{acc}/{seg}")
        };
        crumbs.push((
            seg.to_owned(),
            format!("{base}/browse/{}", encode_path(&acc)),
        ));
    }
    let child_url = |route: &str, name: &str| {
        let full = if path.is_empty() {
            name.to_owned()
        } else {
            format!("{path}/{name}")
        };
        format!("{base}/{route}/{}", encode_path(&full))
    };
    let pages = view.total_children.div_ceil(PAGE_SIZE);

    Ok(views::page(
        Some(&view.site),
        title,
        Nav::Raw,
        html! {
            (views::breadcrumbs(&crumbs))
            div.title-row {
                h1.break { (title) }
                (badge(view.kind, "neutral"))
                @if !path.is_empty() {
                    a.button href={ (base) "/json/" (encode_path(&path)) } { "JSON" }
                }
            }

            @if view.kind == "canvas" {
                figure.canvas {
                    img src={ (base) "/image/" (encode_path(&path)) } alt={ "Image " (title) };
                }
            }
            @if let Some(value) = &view.value {
                div.card { pre.value { (views::value_text(value)) } }
            }

            @if view.total_children > 0 {
                p.muted.small {
                    (view.total_children) " entries"
                    @if pages > 1 { " · page " (q.page + 1) " of " (pages) }
                }
                ul.nodes {
                    @for c in &view.children {
                        li {
                            @if c.has_children {
                                a.node href=(child_url("browse", &c.name)) {
                                    span.name { (c.name) }
                                    (badge(c.kind, "neutral"))
                                    @if let Some(p) = &c.preview { span.label { (p) } }
                                    @if c.kind == "canvas" {
                                        img.thumb src=(child_url("image", &c.name)) alt="" loading="lazy";
                                    }
                                }
                            } @else {
                                div.node {
                                    span.name { (c.name) }
                                    (badge(c.kind, "neutral"))
                                    @if let Some(p) = &c.preview { span.preview { (p) } }
                                }
                            }
                        }
                    }
                }
                @if pages > 1 {
                    nav.pager aria-label="Pages" {
                        @if q.page > 0 { a.button href={ "?page=" (q.page - 1) } { "← Previous" } }
                        @if q.page + 1 < pages { a.button href={ "?page=" (q.page + 1) } { "Next →" } }
                    }
                }
            } @else if view.value.is_none() && view.kind != "canvas" {
                p.empty { "This node is empty." }
            }
        },
    )
    .into_response())
}

/// PNG of a canvas node. Snapshots never change, so cache aggressively.
pub async fn image(
    State(state): State<AppState>,
    Path((version, path)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let png = state
        .run(move |ctx| {
            let v = ctx.version(&version)?;
            let node = ctx.tree(&v)?.get(&path).map_err(AppError::NotFound)?;
            let img = wz::canvas_image(&node).map_err(AppError::NotFound)?;
            let mut buf = Cursor::new(Vec::new());
            img.write_to(&mut buf, image::ImageFormat::Png)
                .map_err(|e| AppError::Internal(e.into()))?;
            Ok(buf.into_inner())
        })
        .await?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        png,
    ))
}

/// A node as JSON, like `datamine show`.
pub async fn json(
    State(state): State<AppState>,
    Path((version, path)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let value = state
        .run(move |ctx| {
            let v = ctx.version(&version)?;
            let node = ctx.tree(&v)?.get(&path).map_err(AppError::NotFound)?;
            wz::parse_to_depth(&node, 3)?;
            Ok(wz::node_to_json(
                &node,
                JsonOptions {
                    content_hashes: false,
                    max_depth: Some(3),
                },
            ))
        })
        .await?;
    Ok(axum::Json(value))
}
