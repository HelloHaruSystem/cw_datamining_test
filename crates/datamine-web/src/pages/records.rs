//! Record-level pages: patch diff, search and history.
//! CLI equivalents: `datamine diff` (with --category/--job/--data-only),
//! `datamine search`, `datamine history`.

use axum::extract::{Query, State};
use datamine_core::db::{RecordFilter, Version};
use datamine_core::diff::{self, Change, FieldChange, Status};
use datamine_core::extract;
use datamine_core::facets::{Category, Detail, FacetFilter};
use datamine_core::jobs::{self, Branch};
use maud::{Markup, html};
use serde::Deserialize;
use serde_json::Value;

use crate::error::AppError;
use crate::site::Site;
use crate::state::AppState;
use crate::views::{self, Nav, badge, encode_path, encode_query, icon};

const SEARCH_LIMIT: usize = 200;
const DIFF_LIMIT_PER_CATEGORY: usize = 300;

// ---- diff -----------------------------------------------------------------

/// Filters are comma-separated lists so links stay short and readable,
/// e.g. `/diff?cat=skills&job=warrior&change=modified`.
#[derive(Deserialize, Default, Clone)]
pub struct DiffQuery {
    from: Option<String>,
    to: Option<String>,
    v: Option<String>,
    #[serde(default)]
    cat: String,
    #[serde(default)]
    job: String,
    #[serde(default)]
    change: String,
    /// "1" to include raw file/.img changes.
    #[serde(default)]
    raw: String,
}

impl DiffQuery {
    fn list(s: &str) -> Vec<&str> {
        s.split(',')
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .collect()
    }

    fn has(list: &str, value: &str) -> bool {
        Self::list(list).contains(&value)
    }

    /// URL with `value` toggled in the list chosen by `field`.
    fn toggle(&self, field: &str, value: &str) -> String {
        let mut q = self.clone();
        let target = match field {
            "cat" => &mut q.cat,
            "job" => &mut q.job,
            "change" => &mut q.change,
            _ => &mut q.raw,
        };
        let mut items: Vec<String> = Self::list(target).into_iter().map(str::to_owned).collect();
        if let Some(pos) = items.iter().position(|x| x == value) {
            items.remove(pos);
        } else {
            items.push(value.to_owned());
        }
        *target = items.join(",");
        q.url()
    }

    fn url(&self) -> String {
        let mut parts = Vec::new();
        for (k, v) in [
            ("from", self.from.as_deref().unwrap_or_default()),
            ("to", self.to.as_deref().unwrap_or_default()),
            ("cat", &self.cat),
            ("job", &self.job),
            ("change", &self.change),
            ("raw", &self.raw),
        ] {
            if !v.is_empty() {
                parts.push(format!("{k}={}", encode_query(v)));
            }
        }
        if parts.is_empty() {
            "/diff".into()
        } else {
            format!("/diff?{}", parts.join("&"))
        }
    }
}

pub async fn diff(
    State(state): State<AppState>,
    Query(query): Query<DiffQuery>,
) -> Result<Markup, AppError> {
    let q = query.clone();
    let (site, result) = state
        .run(move |ctx| {
            let to_sel = q.to.clone().or(q.v.clone()).filter(|s| !s.is_empty());
            let site = Site::load(ctx, to_sel.as_deref(), "/diff")?;
            if site.versions.len() < 2 {
                return Ok((site, None));
            }
            let to = site.version.clone();
            let from = match q.from.as_deref().filter(|s| !s.is_empty()) {
                Some(sel) => ctx.version(sel)?,
                None => match site.versions.iter().take_while(|v| v.id != to.id).last() {
                    Some(prev) => prev.clone(),
                    None => ctx.store.resolve("baseline")?,
                },
            };
            if from.id == to.id {
                return Err(AppError::BadRequest("Pick two different versions.".into()));
            }
            let mut cs = diff::changeset(ctx.store, &from, &to, &RecordFilter::default())?;
            let raw_count = cs
                .changes
                .iter()
                .filter(|c| c.facets.detail == Detail::Raw)
                .count();
            if !DiffQuery::has(&q.raw, "1") {
                cs.retain(&FacetFilter {
                    data_only: true,
                    ..Default::default()
                });
            }
            Ok((site, Some((from, to, cs, raw_count))))
        })
        .await?;

    let Some((from, to, cs, raw_count)) = result else {
        return Ok(views::page(
            Some(&site),
            "Patch diff",
            Nav::Diff,
            html! {
                h1 { "Patch diff" }
                div.empty {
                    p { "Diffs need at least two versions." }
                    p.muted { "Import the next build with " code { "datamine import" } " and come back." }
                }
            },
        ));
    };

    // Facet filters from the query.
    let mut filter = FacetFilter::default();
    for c in DiffQuery::list(&query.cat) {
        let _ = filter.add_category(c);
    }
    for j in DiffQuery::list(&query.job) {
        let _ = filter.add_job(j);
    }
    let statuses: Vec<Status> = DiffQuery::list(&query.change)
        .into_iter()
        .filter_map(|s| match s {
            "added" => Some(Status::Added),
            "removed" => Some(Status::Removed),
            "modified" => Some(Status::Modified),
            _ => None,
        })
        .collect();
    let status_ok = |c: &Change| statuses.is_empty() || statuses.contains(&c.status);

    // Counts for each chip, given the *other* active filters.
    let cat_count = |cat: Category| {
        let f = FacetFilter {
            categories: vec![cat],
            ..filter.clone()
        };
        cs.changes
            .iter()
            .filter(|c| f.matches(&c.facets) && status_ok(c))
            .count()
    };
    let branch_count = |b: Branch| {
        let f = FacetFilter {
            branches: vec![b],
            jobs: vec![],
            ..filter.clone()
        };
        cs.changes
            .iter()
            .filter(|c| f.matches(&c.facets) && status_ok(c))
            .count()
    };
    let status_count = |s: Status| {
        cs.changes
            .iter()
            .filter(|c| filter.matches(&c.facets) && c.status == s)
            .count()
    };
    let shown: Vec<&Change> = cs
        .changes
        .iter()
        .filter(|c| filter.matches(&c.facets) && status_ok(c))
        .collect();
    let include_raw = DiffQuery::has(&query.raw, "1");

    let chip = |field: &str, value: &str, label: &str, count: usize| {
        let active = match field {
            "cat" => DiffQuery::has(&query.cat, value),
            "job" => DiffQuery::has(&query.job, value),
            "change" => DiffQuery::has(&query.change, value),
            _ => include_raw,
        };
        html! {
            a.chip.filter href=(query.toggle(field, value)) aria-pressed=(if active { "true" } else { "false" })
                .is-empty[count == 0 && !active] {
                (label) " " span.chip-count { (count) }
            }
        }
    };

    Ok(views::page(
        Some(&site),
        "Patch diff",
        Nav::Diff,
        html! {
            h1 { "Patch diff" }
            form.filters method="get" action="/diff" {
                (version_select("from", "From", &site.versions, Some(&from)))
                (version_select("to", "To", &site.versions, Some(&to)))
                @for (k, v) in [("cat", &query.cat), ("job", &query.job), ("change", &query.change), ("raw", &query.raw)] {
                    @if !v.is_empty() { input type="hidden" name=(k) value=(v); }
                }
                button.primary type="submit" { "Compare" }
            }

            div.facets {
                div.facet-row {
                    span.facet-label { "Category" }
                    @for c in Category::ALL {
                        @let n = cat_count(c);
                        @if n > 0 || DiffQuery::has(&query.cat, c.slug()) { (chip("cat", c.slug(), c.name(), n)) }
                    }
                }
                div.facet-row {
                    span.facet-label { "Job" }
                    @for b in Branch::ALL {
                        @let n = branch_count(b);
                        @if n > 0 || DiffQuery::has(&query.job, b.slug()) { (chip("job", b.slug(), b.name(), n)) }
                    }
                }
                div.facet-row {
                    span.facet-label { "Change" }
                    (chip("change", "added", "Added", status_count(Status::Added)))
                    (chip("change", "removed", "Removed", status_count(Status::Removed)))
                    (chip("change", "modified", "Changed", status_count(Status::Modified)))
                }
                div.facet-row {
                    span.facet-label { "Raw" }
                    (chip("raw", "1", "Include file and .img changes", raw_count))
                    @if !query.cat.is_empty() || !query.job.is_empty() || !query.change.is_empty() {
                        a.small href=(DiffQuery { cat: String::new(), job: String::new(), change: String::new(), ..query.clone() }.url()) { "Clear filters" }
                    }
                }
            }

            p.muted.small { (shown.len()) " changes from " strong { (from.label) } " to " strong { (to.label) } }

            @if shown.is_empty() {
                p.empty { "No changes match these filters." }
            }
            @for cat in Category::ALL {
                @let in_cat: Vec<&&Change> = shown.iter().filter(|c| c.facets.category == cat).collect();
                @if !in_cat.is_empty() {
                    section {
                        h2 { (cat.name()) " " span.muted { "(" (in_cat.len()) ")" } }
                        ul.records {
                            @for c in in_cat.iter().take(DIFF_LIMIT_PER_CATEGORY) { (change_card(&site, c, &from, &to)) }
                        }
                        @if in_cat.len() > DIFF_LIMIT_PER_CATEGORY {
                            p.muted { (in_cat.len() - DIFF_LIMIT_PER_CATEGORY) " more not shown. Narrow the filters or use "
                                code { "datamine diff --category " (cat.slug()) " --limit 0" } "." }
                        }
                    }
                }
            }
        },
    ))
}

fn change_card(site: &Site, c: &Change, from: &Version, to: &Version) -> Markup {
    let (label, tone) = match c.status {
        Status::Added => ("added", "added"),
        Status::Removed => ("removed", "removed"),
        Status::Modified => ("changed", "changed"),
    };
    // Icons come from the side that has the record.
    let side = if c.status == Status::Removed {
        from
    } else {
        to
    };
    let icon_path = (c.kind == extract::skills::KIND)
        .then(|| {
            let job = jobs::of_skill(c.key.parse().ok()?);
            let has_icon = c.new.as_ref().or(c.old.as_ref())?.get("icon")?.as_bool()?;
            has_icon.then(|| format!("Skill/{job:03}.img/skill/{}/icon", c.key))
        })
        .flatten();
    html! {
        li.card.change {
            div.change-head {
                @if let Some(p) = &icon_path { (icon(side, Some(p), "md")) }
                div.change-title {
                    @if let Some(t) = c.title() { strong { (t) } " " }
                    (record_link(site, &c.kind, &c.key))
                    @if let Some(j) = c.facets.job { " " span.chip { (jobs::name(j)) } }
                }
                (badge(label, tone))
            }
            @match c.status {
                Status::Added => (record_summary(&c.kind, c.new.as_ref())),
                Status::Removed => (record_summary(&c.kind, c.old.as_ref())),
                Status::Modified => (field_table(&c.fields)),
            }
        }
    }
}

/// Readable summary of a whole record, for added and removed changes.
fn record_summary(kind: &str, data: Option<&Value>) -> Markup {
    if kind == extract::skills::KIND
        && let Some(d) = data
    {
        let skill = datamine_core::catalog::Skill::from_record("", d);
        return html! {
            @if let Some(desc) = &skill.desc { p.small.flush { (views::summary(desc)) } }
            div.chips {
                span.chip { "Max level " (skill.max_level) }
                @for (id, lv) in &skill.req { span.chip { "Needs " (id) " " (lv) } }
            }
        };
    }
    views::data_list(data)
}

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
    let text = query.q.trim().to_owned();
    let v = query.v.clone();
    let (site, kinds, hits) = state
        .run(move |ctx| {
            let site = Site::load(ctx, v.as_deref(), "/search")?;
            let kinds = ctx.store.db().kind_counts(site.version.id)?;
            let hits = if text.is_empty() && kind.is_none() {
                vec![]
            } else {
                let filter = RecordFilter {
                    kind: kind.as_deref(),
                };
                ctx.store
                    .db()
                    .search(site.version.id, &text, &filter, SEARCH_LIMIT)?
            };
            Ok((site, kinds, hits))
        })
        .await?;

    let searched =
        !query.q.trim().is_empty() || query.kind.as_deref().is_some_and(|k| !k.is_empty());
    Ok(views::page(
        Some(&site),
        "Search",
        Nav::Search,
        html! {
            h1 { "Search" }
            form.filters method="get" action="/search" role="search" {
                @if site.pinned { input type="hidden" name="v" value=(site.version.label); }
                label.grow {
                    span { "Text" }
                    input type="search" name="q" value=(query.q) placeholder="Name, description or id" autofocus;
                }
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
                                div.change-head {
                                    div.change-title {
                                        @if let Some(name) = h.data.as_ref().and_then(|d| d.get("name").or(d.get("mapName"))).and_then(Value::as_str) {
                                            strong { (name) } " "
                                        }
                                        (record_link(&site, &h.kind, &h.key))
                                    }
                                    span.kind { (h.kind) }
                                }
                                @if h.kind != extract::skills::KIND { (views::data_list(h.data.as_ref())) }
                            }
                        }
                    }
                }
            }
        },
    ))
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
    let (site, hist) = state
        .run(move |ctx| {
            let site = Site::load(ctx, None, "/history")?;
            let filter = RecordFilter {
                kind: kind.as_deref(),
            };
            Ok((site, diff::history(ctx.store, &key, &filter)?))
        })
        .await?;

    Ok(views::page(
        Some(&site),
        &format!("History of {}", query.key),
        Nav::Search,
        html! {
            h1.break { "History of " code { (query.key) } }
            @if hist.is_empty() {
                p.empty { "No records match this key." }
            }
            @for h in &hist {
                section.card {
                    div.change-head { div.change-title { (record_link(&site, &h.kind, &h.key)) } span.kind { (h.kind) } }
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

/// Link to the best page for a record: skill page, else the tree browser,
/// else its history.
fn record_link(site: &Site, kind: &str, key: &str) -> Markup {
    let href = if kind == extract::skills::KIND {
        site.link(&format!("/skills/{key}"))
    } else if let Some(path) = extract::wz_path(kind, key) {
        format!(
            "/v/{}/browse/{}",
            encode_path(&site.version.label),
            encode_path(&path)
        )
    } else {
        format!(
            "/history?key={}&kind={}",
            encode_query(key),
            encode_query(kind)
        )
    };
    html! { a.key.break href=(href) { (key) } }
}

/// `levels/12/damage` -> `Lv 12 · damage`.
fn pretty_path(path: &str) -> String {
    match path.strip_prefix("levels/").and_then(|r| r.split_once('/')) {
        Some((lv, field)) => format!("Lv {lv} · {field}"),
        None => path.to_owned(),
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
                        td data-label="Field" { code { (pretty_path(&f.path)) } }
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
