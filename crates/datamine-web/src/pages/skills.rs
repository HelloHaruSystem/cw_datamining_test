//! Skills database: jobs overview, skills of a job, skill details.
//! CLI equivalents: `datamine jobs`, `datamine skills --job`, `datamine skill`.

use std::collections::BTreeSet;

use axum::extract::{Path, Query, State};
use datamine_core::catalog::{self, Skill};
use datamine_core::db::RecordFilter;
use datamine_core::diff::{self, Status};
use datamine_core::jobs::{self, Branch, JobId};
use maud::{Markup, html};
use serde_json::Value;

use crate::error::AppError;
use crate::site::{Site, VersionParam};
use crate::state::AppState;
use crate::views::{self, Nav, badge, game_text, icon};

fn job_icon(job: JobId) -> String {
    format!("Skill/{job:03}.img/info/icon")
}

fn adv_label(job: JobId) -> &'static str {
    jobs::advancement(job).map_or("Other", jobs::advancement_label)
}

// ---- /skills --------------------------------------------------------------

pub async fn index(
    State(state): State<AppState>,
    Query(q): Query<VersionParam>,
) -> Result<Markup, AppError> {
    let (site, skills) = state
        .run(move |ctx| {
            let site = Site::load(ctx, q.v.as_deref(), "/skills")?;
            let skills = catalog::skills(ctx.store, site.version.id)?;
            Ok((site, skills))
        })
        .await?;
    let job_ids = catalog::skill_jobs(&skills);
    let count = |job: JobId| {
        skills
            .iter()
            .filter(|s| s.job == job && s.is_learnable())
            .count()
    };

    Ok(views::page(
        Some(&site),
        "Skills",
        Nav::Skills,
        html! {
            div.title-row {
                h1 { "Skills" }
                a.button.primary href=(site.link("/tools/skill-builder")) { "Open skill builder" }
            }
            @if job_ids.is_empty() {
                div.empty {
                    p { "No skill data for this version." }
                    p.muted { "Run " code { "datamine extract " (site.version.label) " --only skills" } "." }
                }
            }
            @for branch in Branch::ALL {
                @let in_branch: Vec<_> = job_ids.iter().copied().filter(|j| jobs::branch(*j) == Some(branch)).collect();
                @if !in_branch.is_empty() {
                    section.branch {
                        h2 { (branch.name()) }
                        div.job-grid {
                            @for job in in_branch {
                                a.job.card href=(site.link(&format!("/skills/job/{job}"))) {
                                    (icon(&site.version, Some(&job_icon(job)), "lg"))
                                    div {
                                        strong { (jobs::name(job)) }
                                        div.muted.small { (adv_label(job)) " · " (count(job)) " skills" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    ))
}

// ---- /skills/job/{job} -----------------------------------------------------

pub async fn job(
    State(state): State<AppState>,
    Path(job): Path<JobId>,
    Query(q): Query<VersionParam>,
) -> Result<Markup, AppError> {
    let path = format!("/skills/job/{job}");
    let (site, skills) = state
        .run(move |ctx| {
            let site = Site::load(ctx, q.v.as_deref(), &path)?;
            let skills: Vec<Skill> = catalog::skills(ctx.store, site.version.id)?
                .into_iter()
                .filter(|s| s.job == job)
                .collect();
            Ok((site, skills))
        })
        .await?;
    if skills.is_empty() {
        return Err(AppError::NotFound(anyhow::anyhow!(
            "no skills for job {job}"
        )));
    }
    let (learnable, hidden): (Vec<_>, Vec<_>) = skills.iter().partition(|s| s.is_learnable());
    let line: Vec<JobId> = jobs::path(job);
    let builder_job = job;

    Ok(views::page(
        Some(&site),
        &jobs::name(job),
        Nav::Skills,
        html! {
            (views::breadcrumbs(&[
                ("Skills".into(), site.link("/skills")),
                (jobs::name(job), String::new()),
            ]))
            div.title-row {
                (icon(&site.version, Some(&job_icon(job)), "lg"))
                div {
                    h1 { (jobs::name(job)) }
                    p.muted.small.flush { (adv_label(job))
                        @if line.len() > 1 {
                            " · "
                            @for (i, j) in line.iter().enumerate() {
                                @if i > 0 { " → " }
                                @if *j == job { (jobs::name(*j)) } @else {
                                    a href=(site.link(&format!("/skills/job/{j}"))) { (jobs::name(*j)) }
                                }
                            }
                        }
                    }
                }
                a.button.primary href=(site.link(&format!("/tools/skill-builder?job={builder_job}"))) { "Plan in builder" }
            }
            ul.skill-list {
                @for s in &learnable { (skill_row(&site, s, &skills)) }
            }
            @if !hidden.is_empty() {
                details.hidden-skills {
                    summary { (hidden.len()) " hidden skills (passive effects and internal skills)" }
                    ul.skill-list {
                        @for s in &hidden { (skill_row(&site, s, &skills)) }
                    }
                }
            }
        },
    ))
}

fn skill_row(site: &Site, s: &Skill, all: &[Skill]) -> Markup {
    html! {
        li {
            a.skill-row.card href=(site.link(&format!("/skills/{}", s.id))) {
                (icon(&site.version, s.icon_path().as_deref(), "md"))
                div.skill-main {
                    div.row-between {
                        strong { (s.label()) }
                        span.muted.small.nowrap { "Max " (s.max_level) }
                    }
                    @if let Some(d) = &s.desc { p.muted.small.flush { (views::summary(d)) } }
                    @if !s.req.is_empty() {
                        div.chips {
                            @for (id, lv) in &s.req {
                                span.chip { "Needs " (req_name(id, all)) " " (lv) }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn req_name(id: &str, all: &[Skill]) -> String {
    all.iter()
        .find(|s| s.id == id)
        .map_or_else(|| id.to_owned(), Skill::label)
}

// ---- /skills/{id} ----------------------------------------------------------

pub async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<VersionParam>,
) -> Result<Markup, AppError> {
    let path = format!("/skills/{id}");
    let (site, skill, all, history) = state
        .run(move |ctx| {
            let site = Site::load(ctx, q.v.as_deref(), &path)?;
            let all = catalog::skills(ctx.store, site.version.id)?;
            let skill = all
                .iter()
                .find(|s| s.id == id)
                .cloned()
                .ok_or_else(|| AppError::NotFound(anyhow::anyhow!("no skill {id}")))?;
            let history = diff::history(
                ctx.store,
                &id,
                &RecordFilter {
                    kind: Some(datamine_core::extract::skills::KIND),
                },
            )?;
            Ok((site, skill, all, history))
        })
        .await?;

    // Stat columns that appear on any level, in a stable order.
    let stat_keys: BTreeSet<&str> = skill
        .levels
        .values()
        .flat_map(|m| m.keys().map(String::as_str))
        .filter(|k| *k != "text")
        .collect();
    let events = history
        .first()
        .map(|h| h.events.as_slice())
        .unwrap_or_default();

    Ok(views::page(
        Some(&site),
        &skill.label(),
        Nav::Skills,
        html! {
            (views::breadcrumbs(&[
                ("Skills".into(), site.link("/skills")),
                (jobs::name(skill.job), site.link(&format!("/skills/job/{}", skill.job))),
                (skill.label(), String::new()),
            ]))
            div.skill-hero.card {
                (icon(&site.version, skill.icon_path().as_deref(), "xl"))
                div {
                    h1.flush { (skill.label()) }
                    div.chips {
                        span.chip { (jobs::name(skill.job)) }
                        span.chip { "Max level " (skill.max_level) }
                        span.chip.muted { "ID " (skill.id) }
                        @if skill.invisible { (badge("hidden", "neutral")) }
                    }
                    @if let Some(d) = &skill.desc { p.desc { (game_text(d)) } }
                    @if !skill.req.is_empty() {
                        p.small {
                            strong { "Requires " }
                            @for (i, (rid, lv)) in skill.req.iter().enumerate() {
                                @if i > 0 { ", " }
                                a href=(site.link(&format!("/skills/{rid}"))) { (req_name(rid, &all)) }
                                " level " (lv)
                            }
                        }
                    }
                }
            }

            @if !skill.levels.is_empty() {
                h2 { "Levels" }
                div.table-scroll {
                    table.levels {
                        thead {
                            tr {
                                th.num { "Lv" }
                                th { "Effect" }
                                @for k in &stat_keys { th.num { (k) } }
                            }
                        }
                        tbody {
                            @for (lv, stats) in &skill.levels {
                                tr {
                                    td.num.strong { (lv) }
                                    td.effect { @if let Some(Value::String(t)) = stats.get("text") { (game_text(t)) } }
                                    @for k in &stat_keys {
                                        td.num { @if let Some(v) = stats.get(*k) { (views::value_text(v)) } }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            h2 { "History" }
            @if events.len() <= 1 {
                p.muted { "Unchanged since it was first seen"
                    @if let Some(e) = events.first() { " in " (e.version) } "." }
            } @else {
                ol.timeline {
                    @for e in events {
                        li {
                            strong { (e.version) } " "
                            @match e.status {
                                Status::Added => (badge("added", "added")),
                                Status::Removed => (badge("removed", "removed")),
                                Status::Modified => (badge("changed", "changed")),
                            }
                            @if !e.fields.is_empty() {
                                span.muted.small { " " (e.fields.len()) " fields" }
                            }
                        }
                    }
                }
                p { a href={ "/history?kind=skill&key=" (skill.id) } { "Full change history" } }
            }
            p.small.muted {
                a href=(site.raw("browse", &format!("Skill/{:03}.img/skill/{}", skill.job, skill.id))) { "View raw data" }
            }
        },
    ))
}
