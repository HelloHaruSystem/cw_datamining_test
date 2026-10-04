//! `/tools/skill-builder`: plan SP for a job and level.
//! CLI equivalent: `datamine sp --job <job> --level <n> --build <id:lv,...>`.
//!
//! The server renders every skill card and embeds the data the page needs,
//! including SP pool totals for every level computed by `datamine_core::sp`,
//! so `assets/builder.js` never re-implements the SP rules.

use std::collections::BTreeMap;

use axum::extract::{Query, State};
use datamine_core::catalog::{self, Skill};
use datamine_core::jobs::{self, Branch, JobId};
use datamine_core::sp::SpRules;
use maud::{Markup, PreEscaped, html};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::AppError;
use crate::site::Site;
use crate::state::AppState;
use crate::views::{self, Nav, PageOpts, game_text, icon};

#[derive(Deserialize, Default)]
pub struct BuilderQuery {
    v: Option<String>,
    job: Option<String>,
    lv: Option<u32>,
}

pub async fn builder(
    State(state): State<AppState>,
    Query(q): Query<BuilderQuery>,
) -> Result<Markup, AppError> {
    let (site, skills) = state
        .run(move |ctx| {
            let site = Site::load(ctx, q.v.as_deref(), "/tools/skill-builder")?;
            let skills = catalog::skills(ctx.store, site.version.id)?;
            Ok((site, skills))
        })
        .await?;
    let rules = SpRules::default();
    let available = catalog::skill_jobs(&skills);
    let target = q
        .job
        .as_deref()
        .and_then(jobs::find)
        .filter(|j| available.contains(j))
        .or_else(|| available.iter().copied().find(|j| *j != 0))
        .ok_or_else(|| AppError::NotFound(anyhow::anyhow!("this version has no skill data")))?;
    let level =
        q.lv.unwrap_or_else(|| default_level(&rules, target))
            .clamp(rules.min_level(target), rules.level_cap);
    let path = jobs::path(target);
    let path_skills: Vec<&Skill> = skills
        .iter()
        .filter(|s| s.is_learnable() && path.contains(&s.job))
        .collect();

    let data = builder_data(&site, &rules, target, level, &path, &path_skills);
    // `</` cannot appear inside the script element.
    let data_json = data.to_string().replace("</", "<\\/");

    Ok(views::page_with(
        Some(&site),
        PageOpts {
            title: "Skill builder",
            nav: Nav::Builder,
            scripts: &["builder.js"],
        },
        html! {
            div.title-row {
                h1 { "Skill builder" }
            }
            form.filters.builder-controls method="get" action="/tools/skill-builder" data-builder-form {
                @if site.pinned { input type="hidden" name="v" value=(site.version.label); }
                label {
                    span { "Job" }
                    select name="job" data-builder-job {
                        @for branch in Branch::ALL {
                            @let opts: Vec<_> = available.iter().copied()
                                .filter(|j| *j != 0 && jobs::branch(*j) == Some(branch)).collect();
                            @if !opts.is_empty() {
                                optgroup label=(branch.name()) {
                                    @for j in opts {
                                        option value=(j) selected[j == target] { (jobs::name(j)) }
                                    }
                                }
                            }
                        }
                    }
                }
                label.grow {
                    span { "Level " output data-builder-level-out { (level) } }
                    input type="range" name="lv" min=(rules.min_level(target)) max=(rules.level_cap)
                        value=(level) data-builder-level;
                }
                noscript { button.primary type="submit" { "Update" } }
            }

            noscript {
                p.notice { "The builder needs JavaScript to add skill points. Without it you can still browse the skills and SP totals." }
            }

            div.pools data-builder-pools {
                @for (job, total) in rules.pools(target, level) {
                    a.pool href={ "#job-" (job) } data-pool=(job) {
                        span.pool-name { (jobs::name(job)) }
                        span.pool-count { span data-pool-spent { "0" } " / " span data-pool-total { (total) } }
                        span.meter { span.meter-fill data-pool-meter {} }
                    }
                }
            }
            div.builder-actions {
                button type="button" data-builder-share hidden { "Copy share link" }
                button type="button" data-builder-reset-all hidden { "Reset all" }
                span.muted.small role="status" data-builder-status {}
            }
            div.problems role="alert" data-builder-problems hidden {}

            @for job in &path {
                @let job_skills: Vec<&&Skill> = path_skills.iter().filter(|s| s.job == *job).collect();
                section.builder-job id={ "job-" (job) } {
                    div.row-between {
                        h2 { (jobs::name(*job)) }
                        button.small type="button" data-builder-reset=(job) hidden { "Reset" }
                    }
                    @if job_skills.is_empty() {
                        p.muted { "No skills." }
                    }
                    ul.builder-grid {
                        @for s in job_skills { (skill_card(&site, s, &path_skills)) }
                    }
                }
            }
            p.muted.small {
                "SP: beginners get 1 per level for levels 2–7; each job advancement gives 1, then 3 per level. "
                "Magicians advance at 8, other 1st jobs at 10, 2nd jobs at 30, 3rd at 70. SP from a job can only be spent on that job's skills."
            }
            script type="application/json" #builder-data { (PreEscaped(data_json)) }
        },
    ))
}

fn default_level(rules: &SpRules, job: JobId) -> u32 {
    let level = match jobs::get(job).map(|j| j.advancement) {
        Some(1) => 30,
        Some(2) => 70,
        Some(3) => 120,
        _ => rules.min_level(job),
    };
    level.min(rules.level_cap)
}

fn skill_card(site: &Site, s: &Skill, all: &[&Skill]) -> Markup {
    let first = s
        .levels
        .get(&1)
        .and_then(|l| l.get("text"))
        .and_then(Value::as_str);
    html! {
        li.builder-skill.card data-skill=(s.id) {
            div.builder-head {
                (icon(&site.version, s.icon_path().as_deref(), "md"))
                div.builder-name {
                    a href=(site.link(&format!("/skills/{}", s.id))) { strong { (s.label()) } }
                    div.small.muted {
                        span.level-count { span data-skill-level { "0" } " / " (s.max_level) }
                        @for (rid, lv) in &s.req {
                            " · needs "
                            (all.iter().find(|o| o.id == *rid).map_or_else(|| rid.clone(), |o| o.label()))
                            " " (lv)
                        }
                    }
                }
                div.stepper {
                    button.icon-button type="button" data-step="-1" aria-label={ "Remove a point from " (s.label()) } disabled { "−" }
                    button.icon-button type="button" data-step="1" aria-label={ "Add a point to " (s.label()) } disabled { "+" }
                    button.small type="button" data-step="max" aria-label={ "Max " (s.label()) } disabled { "Max" }
                }
            }
            p.small.effect data-skill-effect {
                @if let Some(t) = first { span.muted { "Lv 1: " } (game_text(t)) }
            }
        }
    }
}

fn builder_data(
    site: &Site,
    rules: &SpRules,
    target: JobId,
    level: u32,
    path: &[JobId],
    skills: &[&Skill],
) -> Value {
    // pools[level - 1][i] = SP total of path[i] at that level.
    let pools: Vec<Vec<u32>> = (1..=rules.level_cap)
        .map(|lv| {
            rules
                .pools(target, lv)
                .into_iter()
                .map(|(_, total)| total)
                .collect()
        })
        .collect();
    let skills: Vec<Value> = skills
        .iter()
        .map(|s| {
            let texts: BTreeMap<String, String> = s
                .levels
                .iter()
                .filter_map(|(lv, stats)| {
                    Some((
                        lv.to_string(),
                        views::game_text(stats.get("text")?.as_str()?).into_string(),
                    ))
                })
                .collect();
            json!({
                "id": s.id,
                "job": s.job,
                "name": s.label(),
                "max": s.max_level,
                "req": s.req,
                "texts": texts,
            })
        })
        .collect();
    json!({
        "job": target,
        "level": level,
        "minLevel": rules.min_level(target),
        "path": path,
        "names": path.iter().map(|j| (j.to_string(), jobs::name(*j))).collect::<BTreeMap<_, _>>(),
        "pools": pools,
        "skills": skills,
        "version": site.version.label,
        "pinned": site.pinned,
    })
}
