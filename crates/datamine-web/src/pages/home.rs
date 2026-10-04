//! `/`: overview of the selected version (newest by default).
//! CLI equivalents: `datamine info`, `datamine diff --from previous`.

use axum::extract::{Query, State};
use datamine_core::catalog;
use datamine_core::db::RecordFilter;
use datamine_core::diff;
use datamine_core::facets::{Category, FacetFilter};
use maud::{Markup, html};

use crate::error::AppError;
use crate::site::{Site, VersionParam};
use crate::state::AppState;
use crate::views::{self, Nav, badge, encode_query, short_time};

struct Section {
    title: &'static str,
    href: String,
    count: i64,
    blurb: &'static str,
}

pub async fn home(
    State(state): State<AppState>,
    Query(q): Query<VersionParam>,
) -> Result<Markup, AppError> {
    let (site, sections, learnable, changes) = state
        .run(move |ctx| {
            let site = Site::load(ctx, q.v.as_deref(), "/")?;
            let v = &site.version;
            let counts = ctx.store.db().kind_counts(v.id)?;
            let count = |kinds: &[&str]| -> i64 {
                counts
                    .iter()
                    .filter(|(k, _)| kinds.contains(&k.as_str()))
                    .map(|(_, n)| n)
                    .sum()
            };
            let search = |kind: &str| site.link(&format!("/search?kind={}", encode_query(kind)));
            let sections = vec![
                Section {
                    title: "Equipment",
                    href: search("string/Eqp"),
                    count: count(&["string/Eqp"]),
                    blurb: "Weapons, armor and accessories",
                },
                Section {
                    title: "Use items",
                    href: search("string/Consume"),
                    count: count(&["string/Consume"]),
                    blurb: "Potions, scrolls and other consumables",
                },
                Section {
                    title: "Etc & setup",
                    href: search("string/Etc"),
                    count: count(&["string/Etc", "string/Ins"]),
                    blurb: "Quest items, materials and chairs",
                },
                Section {
                    title: "Monsters",
                    href: search("string/Mob"),
                    count: count(&["string/Mob"]),
                    blurb: "Every monster by name",
                },
                Section {
                    title: "NPCs",
                    href: search("string/Npc"),
                    count: count(&["string/Npc"]),
                    blurb: "Shops, quest givers and more",
                },
                Section {
                    title: "Maps",
                    href: search("string/Map"),
                    count: count(&["string/Map"]),
                    blurb: "Towns, fields and dungeons",
                },
            ];
            let learnable = catalog::skills(ctx.store, v.id)?
                .iter()
                .filter(|s| s.is_learnable())
                .count();

            // What changed since the version before this one.
            let prev = site
                .versions
                .iter()
                .take_while(|p| p.id != v.id)
                .last()
                .cloned();
            let changes = match prev {
                Some(prev) => {
                    let mut cs = diff::changeset(ctx.store, &prev, v, &RecordFilter::default())?;
                    cs.retain(&FacetFilter {
                        data_only: true,
                        ..Default::default()
                    });
                    let mut per_cat: Vec<(Category, usize)> = Category::ALL
                        .iter()
                        .map(|c| {
                            (
                                *c,
                                cs.changes
                                    .iter()
                                    .filter(|ch| ch.facets.category == *c)
                                    .count(),
                            )
                        })
                        .filter(|(_, n)| *n > 0)
                        .collect();
                    per_cat.sort_by_key(|c| std::cmp::Reverse(c.1));
                    Some((prev, per_cat))
                }
                None => None,
            };
            Ok((site, sections, learnable, changes))
        })
        .await?;
    let v = &site.version;

    Ok(views::page(
        Some(&site),
        &v.label,
        Nav::Home,
        html! {
            section.hero {
                div.hero-text {
                    p.eyebrow {
                        (badge(&v.channel, "accent"))
                        @if site.is_latest() { " " (badge("newest", "neutral")) }
                    }
                    h1 { (v.label) }
                    p.muted {
                        @if let Some(b) = &v.build_time { "Built " (short_time(b)) " · " }
                        "Imported " (short_time(&v.imported_at))
                    }
                }
                form.hero-search method="get" action="/search" role="search" {
                    label.sr-only for="hero-q" { "Search this version" }
                    input #hero-q type="search" name="q" placeholder="Search items, skills, monsters, maps…";
                    @if site.pinned { input type="hidden" name="v" value=(v.label); }
                    button.primary type="submit" { "Search" }
                }
            }

            div.feature-grid {
                a.feature.card href=(site.link("/skills")) {
                    span.feature-kicker { "Database" }
                    h2 { "Skills" }
                    p.muted { (learnable) " skills across every job, with per-level stats." }
                }
                a.feature.card.accent href=(site.link("/tools/skill-builder")) {
                    span.feature-kicker { "Tool" }
                    h2 { "Skill builder" }
                    p.muted { "Plan SP for any job and level, then share the link." }
                }
                a.feature.card href={ "/diff?to=" (encode_query(&v.label)) } {
                    span.feature-kicker { "Patch" }
                    h2 { "What changed" }
                    @match &changes {
                        Some((prev, per_cat)) => {
                            p.muted { "Since " (prev.label) ": "
                                @if per_cat.is_empty() { "no data changes." }
                                @for (i, (c, n)) in per_cat.iter().enumerate() {
                                    @if i > 0 { ", " }
                                    (n) " " (c.name().to_lowercase())
                                }
                            }
                        }
                        None => { p.muted { "This is the first version. Diffs appear once the next build is imported." } }
                    }
                }
            }

            h2 { "Browse the database" }
            div.section-grid {
                @for s in &sections {
                    a.section.card href=(s.href) {
                        div.row-between {
                            strong { (s.title) }
                            span.count { (s.count) }
                        }
                        p.muted.small { (s.blurb) }
                    }
                }
            }
        },
    ))
}
