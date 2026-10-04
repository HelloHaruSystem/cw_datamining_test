//! Record-level pages: search, diff between versions, and history.
//! CLI equivalents: `datamine search`, `datamine diff`, `datamine history`.

use axum::extract::{Query, State};
use datamine_core::db::{RecordFilter, Version};
use datamine_core::diff::{self, Change, FieldChange, Status};
use datamine_core::extract;
use maud::{Markup, html};
use serde::Deserialize;
use serde_json::Value;

use crate::error::AppError;
use crate::state::AppState;
use crate::views::{self, Nav, badge, encode_path, encode_query};

const SEARCH_LIMIT: usize = 200;
const DIFF_LIMIT_PER_KIND: usize = 300;

// ---- search ---------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct SearchQuery {
    #[serde(default)]
    q: String,
    v: Option<String>,
    kind: Option<String>,
}

pub async fn search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Markup, AppError> {
    let kind = query.kind.clone().filter(|k| !k.is_empty());
    let selector = query
        .v
        .clone()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "latest".into());
    let text = query.q.trim().to_owned();
    let (versions, current, kinds, hits) = state
        .run(move |ctx| {
            let versions = ctx.store.db().versions()?;
            if versions.is_empty() {
                return Ok((versions, None, vec![], vec![]));
            }
            let v = ctx.version(&selector)?;
            let kinds = ctx.store.db().kind_counts(v.id)?;
            let hits = if text.is_empty() && kind.is_none() {
                vec![]
            } else {
                let filter = RecordFilter {
                    kind: kind.as_deref(),
                };
                ctx.store.db().search(v.id, &text, &filter, SEARCH_LIMIT)?
            };
            Ok((versions, Some(v), kinds, hits))
        })
        .await?;

    let searched =
        !query.q.trim().is_empty() || query.kind.as_deref().is_some_and(|k| !k.is_empty());
    Ok(views::page(
        "Search",
        Nav::Search,
        html! {
            h1 { "Search" }
            form.filters method="get" action="/search" role="search" {
                label.grow {
                    span { "Text" }
                    input type="search" name="q" value=(query.q) placeholder="Name, description or id" autofocus;
                }
                (version_select("v", "Version", &versions, current.as_ref()))
                label {
                    span { "Kind" }
                    select name="kind" {
                        option value="" { "All kinds" }
                        @for (k, n) in &kinds {
                            option value=(k) selected[query.kind.as_deref() == Some(k.as_str())] { (k) " (" (n) ")" }
                        }
                    }
                }
                button.primary type="submit" { "Search" }
            }

            @if searched {
                @if hits.is_empty() {
                    p.empty { "No matches." }
                } @else {
                    p.muted.small {
                        (hits.len()) " results"
                        @if hits.len() == SEARCH_LIMIT { " (first " (SEARCH_LIMIT) " shown; narrow the search)" }
                    }
                    ul.records {
                        @for h in &hits {
                            li.card {
                                (record_heading(&h.kind, &h.key, current.as_ref()))
                                (views::data_list(h.data.as_ref()))
                            }
                        }
                    }
                }
            }
        },
    ))
}

// ---- diff -----------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct DiffQuery {
    from: Option<String>,
    to: Option<String>,
    kind: Option<String>,
}

pub async fn diff(
    State(state): State<AppState>,
    Query(query): Query<DiffQuery>,
) -> Result<Markup, AppError> {
    let kind = query.kind.clone().filter(|k| !k.is_empty());
    let from_sel = query
        .from
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "baseline".into());
    let to_sel = query
        .to
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "latest".into());
    let (versions, cs) = state
        .run(move |ctx| {
            let versions = ctx.store.db().versions()?;
            if versions.len() < 2 {
                return Ok((versions, None));
            }
            let from = ctx.version(&from_sel)?;
            let to = ctx.version(&to_sel)?;
            if from.id == to.id {
                return Err(AppError::BadRequest("Pick two different versions.".into()));
            }
            let filter = RecordFilter {
                kind: kind.as_deref(),
            };
            Ok((
                versions,
                Some(diff::changeset(ctx.store, &from, &to, &filter)?),
            ))
        })
        .await?;

    Ok(views::page(
        "Diff",
        Nav::Diff,
        html! {
            h1 { "Diff" }
            @match &cs {
                None => {
                    div.empty {
                        p { "Diffs need at least two versions." }
                        p.muted { "Import the next build with " code { "datamine import" } " and come back." }
                    }
                }
                Some(cs) => {
                    form.filters method="get" action="/diff" {
                        (version_select("from", "From", &versions, Some(&cs.from)))
                        (version_select("to", "To", &versions, Some(&cs.to)))
                        label.grow {
                            span { "Kind prefix" }
                            input type="text" name="kind" value=[query.kind.as_deref()] placeholder="e.g. string or string/Eqp";
                        }
                        button.primary type="submit" { "Compare" }
                    }
                    @if cs.changes.is_empty() {
                        p.empty { "No changes between " strong { (cs.from.label) } " and " strong { (cs.to.label) } "." }
                    } @else {
                        table.summary {
                            thead { tr { th { "Kind" } th.num { "Added" } th.num { "Removed" } th.num { "Changed" } } }
                            tbody {
                                @for (k, a, r, m) in cs.summary() {
                                    tr {
                                        td { a href={ "#k-" (k) } { code { (k) } } }
                                        td.num.added { (a) } td.num.removed { (r) } td.num.changed { (m) }
                                    }
                                }
                            }
                        }
                        @for (k, changes) in cs.by_kind() {
                            section #{ "k-" (k) } {
                                h2 { code { (k) } }
                                ul.records {
                                    @for c in changes.iter().take(DIFF_LIMIT_PER_KIND) {
                                        (change_card(c, &cs.to))
                                    }
                                }
                                @if changes.len() > DIFF_LIMIT_PER_KIND {
                                    p.muted { (changes.len() - DIFF_LIMIT_PER_KIND) " more not shown. Filter by kind or use "
                                        code { "datamine diff --kind " (k) " --limit 0" } "." }
                                }
                            }
                        }
                    }
                }
            }
        },
    ))
}

fn change_card(c: &Change, to: &Version) -> Markup {
    let (label, tone) = match c.status {
        Status::Added => ("added", "added"),
        Status::Removed => ("removed", "removed"),
        Status::Modified => ("changed", "changed"),
    };
    html! {
        li.card {
            div.row-between {
                (record_heading(&c.kind, &c.key, Some(to)))
                (badge(label, tone))
            }
            @match c.status {
                Status::Added => (views::data_list(c.new.as_ref())),
                Status::Removed => (views::data_list(c.old.as_ref())),
                Status::Modified => (field_table(&c.fields)),
            }
        }
    }
}

// ---- history --------------------------------------------------------------

#[derive(Deserialize)]
pub struct HistoryQuery {
    key: String,
    kind: Option<String>,
}

pub async fn history(
    State(state): State<AppState>,
    Query(query): Query<HistoryQuery>,
) -> Result<Markup, AppError> {
    let key = query.key.clone();
    let kind = query.kind.clone().filter(|k| !k.is_empty());
    let (hist, latest) = state
        .run(move |ctx| {
            let filter = RecordFilter {
                kind: kind.as_deref(),
            };
            let hist = diff::history(ctx.store, &key, &filter)?;
            let latest = ctx.store.db().versions()?.pop();
            Ok((hist, latest))
        })
        .await?;

    Ok(views::page(
        &format!("History of {}", query.key),
        Nav::Search,
        html! {
            h1.break { "History of " code { (query.key) } }
            @if hist.is_empty() {
                p.empty { "No records match this key." }
            }
            @for h in &hist {
                section.card {
                    (record_heading(&h.kind, &h.key, latest.as_ref()))
                    ol.timeline {
                        @for e in &h.events {
                            li {
                                div.row-between {
                                    strong { (e.version) }
                                    @match e.status {
                                        Status::Added => (badge("added", "added")),
                                        Status::Removed => (badge("removed", "removed")),
                                        Status::Modified => (badge("changed", "changed")),
                                    }
                                }
                                @if e.fields.is_empty() {
                                    @if e.status != Status::Removed { (views::data_list(e.data.as_ref())) }
                                } @else {
                                    (field_table(&e.fields))
                                }
                            }
                        }
                    }
                }
            }
        },
    ))
}

// ---- shared ---------------------------------------------------------------

/// Kind + key, linking to history and (when possible) the tree browser.
fn record_heading(kind: &str, key: &str, version: Option<&Version>) -> Markup {
    let history = format!(
        "/history?key={}&kind={}",
        encode_query(key),
        encode_query(kind)
    );
    let browse = version
        .zip(extract::wz_path(kind, key))
        .map(|(v, path)| format!("/v/{}/browse/{}", encode_path(&v.label), encode_path(&path)));
    html! {
        div.record-head {
            span.kind { (kind) }
            a.key.break href=(history) title="History" { (key) }
            @if let Some(href) = browse {
                a.small href=(href) { "Browse" }
            }
        }
    }
}

fn field_table(fields: &[FieldChange]) -> Markup {
    let cell = |v: &Option<Value>| v.as_ref().map(views::value_text);
    html! {
        table.fields {
            thead { tr { th { "Field" } th { "Before" } th { "After" } } }
            tbody {
                @for f in fields {
                    tr {
                        td data-label="Field" { code { (f.path) } }
                        td.before data-label="Before" {
                            @if let Some(t) = cell(&f.old) { del { (t) } } @else { span.muted { "—" } }
                        }
                        td.after data-label="After" {
                            @if let Some(t) = cell(&f.new) { ins { (t) } } @else { span.muted { "—" } }
                        }
                    }
                }
            }
        }
    }
}

fn version_select(
    name: &str,
    label: &str,
    versions: &[Version],
    current: Option<&Version>,
) -> Markup {
    html! {
        label {
            span { (label) }
            select name=(name) {
                @for v in versions.iter().rev() {
                    option value=(v.label) selected[current.is_some_and(|c| c.id == v.id)] { (v.label) }
                }
            }
        }
    }
}
