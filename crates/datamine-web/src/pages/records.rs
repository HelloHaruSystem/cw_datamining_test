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
/// Changes listed per category before a "show all" link.
const DIFF_LIMIT_PER_CATEGORY: usize = 100;

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
    /// Job advancements: beginner, 1st, 2nd, 3rd, 4th.
    #[serde(default)]
    adv: String,
    #[serde(default)]
    change: String,
    /// "1" to include raw file/.img changes.
    #[serde(default)]
    raw: String,
    /// "hide" to hide text-only changes.
    #[serde(default)]
    text: String,
    /// Category slug whose changes are all listed (no per-category limit).
    #[serde(default)]
    all: String,
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
            "adv" => &mut q.adv,
            "change" => &mut q.change,
            "text" => &mut q.text,
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
            ("adv", &self.adv),
            ("change", &self.change),
            ("raw", &self.raw),
            ("text", &self.text),
            ("all", &self.all),
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
                None => ctx.store.previous(&to)?.ok_or_else(|| {
                    AppError::BadRequest(format!(
                        "{} is the oldest version; pick another one.",
                        to.label
                    ))
                })?,
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
    for a in DiffQuery::list(&query.adv) {
        let _ = filter.add_advancement(a);
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
    let hide_text = DiffQuery::has(&query.text, "hide");
    let status_ok = |c: &Change| {
        (statuses.is_empty() || statuses.contains(&c.status)) && !(hide_text && c.is_text_only())
    };

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
    let adv_count = |adv: u8| {
        let f = FacetFilter {
            advancements: vec![adv],
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
            .filter(|c| !(hide_text && c.is_text_only()))
            .count()
    };
    let text_only_count = cs
        .changes
        .iter()
        .filter(|c| filter.matches(&c.facets) && c.is_text_only())
        .filter(|c| statuses.is_empty() || statuses.contains(&c.status))
        .count();
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
            "adv" => DiffQuery::has(&query.adv, value),
            "change" => DiffQuery::has(&query.change, value),
            "text" => hide_text,
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
                @for (k, v) in [("cat", &query.cat), ("job", &query.job), ("adv", &query.adv), ("change", &query.change), ("raw", &query.raw), ("text", &query.text)] {
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
                    span.facet-label { "Advancement" }
                    @for (a, slug, label) in jobs::ADVANCEMENTS {
                        @let n = adv_count(a);
                        @if n > 0 || DiffQuery::has(&query.adv, slug) { (chip("adv", slug, label, n)) }
                    }
                }
                div.facet-row {
                    span.facet-label { "Change" }
                    (chip("change", "added", "Added", status_count(Status::Added)))
                    (chip("change", "removed", "Removed", status_count(Status::Removed)))
                    (chip("change", "modified", "Changed", status_count(Status::Modified)))
                }
                div.facet-row {
                    span.facet-label { "Text" }
                    (chip("text", "hide", "Hide text-only changes", text_only_count))
                }
                div.facet-row {
                    span.facet-label { "Raw" }
                    (chip("raw", "1", "Include file and .img changes", raw_count))
                    @if !query.cat.is_empty() || !query.job.is_empty() || !query.adv.is_empty() || !query.change.is_empty() {
                        a.small href=(DiffQuery { cat: String::new(), job: String::new(), adv: String::new(), change: String::new(), ..query.clone() }.url()) { "Clear filters" }
                    }
                }
            }

            p.muted.small { (shown.len()) " changes from " strong { (from.label) } " to " strong { (to.label) } }

            @if shown.is_empty() {
                p.empty { "No changes match these filters." }
            }
            @for cat in Category::ALL {
                @let in_cat: Vec<&&Change> = shown.iter().filter(|c| c.facets.category == cat).collect();
                @let limit = if query.all == cat.slug() || DiffQuery::list(&query.cat).len() == 1 {
                    usize::MAX
                } else {
                    DIFF_LIMIT_PER_CATEGORY
                };
                @if !in_cat.is_empty() {
                    section id={ "cat-" (cat.slug()) } {
                        h2 { (cat.name()) " " span.muted { "(" (in_cat.len()) ")" } }
                        ul.records {
                            @for c in in_cat.iter().take(limit) { (change_card(&site, c, &from, &to)) }
                        }
                        @if in_cat.len() > limit {
                            p {
                                a.button href={ (DiffQuery { all: cat.slug().into(), ..query.clone() }.url()) "#cat-" (cat.slug()) } {
                                    "Show all " (in_cat.len()) " " (cat.name().to_lowercase())
                                }
                            }
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

/// Fields grouped for display: per-level changes to the same field
/// (`levels/1/text`, `levels/2/text`, ...) become one group.
enum FieldGroup<'a> {
    Single(&'a FieldChange),
    Levels {
        field: &'a str,
        changes: Vec<(u32, &'a FieldChange)>,
    },
}

fn group_fields(fields: &[FieldChange]) -> Vec<FieldGroup<'_>> {
    let mut out: Vec<FieldGroup> = Vec::new();
    for f in fields {
        let level_field = f
            .path
            .strip_prefix("levels/")
            .and_then(|r| r.split_once('/'))
            .and_then(|(lv, field)| Some((lv.parse::<u32>().ok()?, field)));
        match level_field {
            Some((lv, field)) => {
                let existing = out.iter_mut().find_map(|g| match g {
                    FieldGroup::Levels { field: f2, changes } if *f2 == field => Some(changes),
                    _ => None,
                });
                match existing {
                    Some(changes) => changes.push((lv, f)),
                    None => out.push(FieldGroup::Levels {
                        field,
                        changes: vec![(lv, f)],
                    }),
                }
            }
            None => out.push(FieldGroup::Single(f)),
        }
    }
    for g in &mut out {
        if let FieldGroup::Levels { changes, .. } = g {
            changes.sort_by_key(|(lv, _)| *lv);
        }
    }
    out
}

fn field_row(label: &str, f: &FieldChange) -> Markup {
    let cell = |v: &Option<Value>| v.as_ref().map(views::value_text);
    html! {
        tr {
            td data-label="Field" { code { (label) } }
            td.before data-label="Before" {
                @if let Some(t) = cell(&f.old) { del { (t) } } @else { span.muted { "—" } }
            }
            td.after data-label="After" {
                @if let Some(t) = cell(&f.new) { ins { (t) } } @else { span.muted { "—" } }
            }
        }
    }
}

/// Levels as compact ranges: `1–5, 8, 10–12`.
fn level_ranges(levels: &[u32]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < levels.len() {
        let start = levels[i];
        while i + 1 < levels.len() && levels[i + 1] == levels[i] + 1 {
            i += 1;
        }
        parts.push(if levels[i] == start {
            start.to_string()
        } else {
            format!("{start}–{}", levels[i])
        });
        i += 1;
    }
    parts.join(", ")
}

fn field_table(fields: &[FieldChange]) -> Markup {
    html! {
        table.fields {
            thead { tr { th { "Field" } th { "Before" } th { "After" } } }
            tbody {
                @for g in group_fields(fields) {
                    @match g {
                        FieldGroup::Single(f) => (field_row(&f.path, f)),
                        FieldGroup::Levels { field, changes } if changes.len() < 3 => {
                            @for (lv, f) in &changes { (field_row(&format!("Lv {lv} · {field}"), f)) }
                        }
                        FieldGroup::Levels { field, changes } => {
                            @let levels: Vec<u32> = changes.iter().map(|(lv, _)| *lv).collect();
                            (field_row(&format!("Lv {} · {field} ({} levels)", level_ranges(&levels), changes.len()), changes[0].1))
                            tr.group-detail {
                                td colspan="3" {
                                    details {
                                        summary { "Show all " (changes.len()) " levels" }
                                        table.fields {
                                            tbody {
                                                @for (lv, f) in &changes { (field_row(&format!("Lv {lv}"), f)) }
                                            }
                                        }
                                    }
                                }
                            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_ranges_compact() {
        assert_eq!(level_ranges(&[1, 2, 3, 5, 7, 8]), "1–3, 5, 7–8");
        assert_eq!(level_ranges(&[4]), "4");
    }
}
