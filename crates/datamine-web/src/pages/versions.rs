//! `/versions` (all versions) and `/v/{version}` (one version's details).
//! CLI equivalents: `datamine versions`, `datamine info <version>`.

use axum::extract::{Path, State};
use maud::{Markup, html};

use crate::error::AppError;
use crate::site::Site;
use crate::state::AppState;
use crate::views::{self, Nav, badge, encode_path, encode_query, short_time};

pub async fn list(State(state): State<AppState>) -> Result<Markup, AppError> {
    let (site, versions, baseline) = state
        .run(|ctx| {
            let site = Site::load(ctx, None, "/versions").ok();
            Ok((
                site,
                ctx.store.db().versions()?,
                ctx.store.baseline()?.map(|b| b.id),
            ))
        })
        .await?;

    Ok(views::page(
        site.as_ref(),
        "Versions",
        Nav::Versions,
        html! {
            h1 { "Versions" }
            @if versions.is_empty() {
                div.empty {
                    p { "No versions imported yet." }
                    p.muted { "Run " code { "datamine import <client> --label <name>" } " to add one." }
                }
            } @else {
                table.stack {
                    thead { tr { th { "Version" } th { "Channel" } th { "Build" } th { "Imported" } th { span.sr-only { "Actions" } } } }
                    tbody {
                        @for v in versions.iter().rev() {
                            tr {
                                td data-label="Version" {
                                    div.cell {
                                        a.strong href={ "/v/" (encode_path(&v.label)) } { (v.label) }
                                        @if Some(v.id) == baseline { " " (badge("baseline", "accent")) }
                                        @if let Some(note) = &v.note { div.muted.small { (note) } }
                                    }
                                }
                                td data-label="Channel" { (badge(&v.channel, "neutral")) }
                                td data-label="Build" { (v.build_time.as_deref().map(short_time).unwrap_or_else(|| "—".into())) }
                                td data-label="Imported" { (short_time(&v.imported_at)) }
                                td.actions {
                                    a.button href={ "/?v=" (encode_query(&v.label)) } { "Open" }
                                }
                            }
                        }
                    }
                }
            }
        },
    ))
}

pub async fn detail(
    State(state): State<AppState>,
    Path(selector): Path<String>,
) -> Result<Markup, AppError> {
    let path = format!("/v/{}", encode_path(&selector));
    let (site, v, is_baseline, extractions, kinds) = state
        .run(move |ctx| {
            let site = Site::load(ctx, Some(&selector), &path)?;
            let v = site.version.clone();
            let is_baseline = ctx.store.baseline()?.is_some_and(|b| b.id == v.id);
            let extractions = ctx.store.db().extractions(v.id)?;
            let kinds = ctx.store.db().kind_counts(v.id)?;
            Ok((site, v, is_baseline, extractions, kinds))
        })
        .await?;
    let label = encode_path(&v.label);

    Ok(views::page(
        Some(&site),
        &v.label,
        Nav::Versions,
        html! {
            (views::breadcrumbs(&[("Versions".into(), "/versions".into()), (v.label.clone(), String::new())]))
            div.title-row {
                h1 { (v.label) }
                @if is_baseline { (badge("baseline", "accent")) }
                a.button.primary href={ "/v/" (label) "/browse" } { "Browse data" }
            }
            dl.kv.card {
                dt { "Channel" } dd { (v.channel) }
                dt { "Build time" } dd { (v.build_time.as_deref().unwrap_or("—")) }
                dt { "Imported" } dd { (v.imported_at) }
                @if let Some(h) = &v.manifest_hash { dt { "Manifest" } dd { code.wrap { (h) } } }
                @if let Some(n) = &v.note { dt { "Note" } dd { (n) } }
            }
            div.grid-2 {
                section {
                    h2 { "Extractions" }
                    ul.plain {
                        @for e in &extractions {
                            li.row-between {
                                span { (e.extractor) }
                                span.muted { (e.record_count) " records" }
                            }
                        }
                    }
                }
                section {
                    h2 { "Records by kind" }
                    ul.plain {
                        @for (kind, count) in &kinds {
                            li.row-between {
                                a href={ "/search?v=" (encode_query(&v.label)) "&kind=" (encode_query(kind)) } { code { (kind) } }
                                span.muted { (count) }
                            }
                        }
                    }
                }
            }
        },
    ))
}
