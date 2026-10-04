//! Page shell and small reusable pieces of markup.

use maud::{DOCTYPE, Markup, PreEscaped, html};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde_json::Value;

use datamine_core::db::Version;

use crate::assets;
use crate::site::Site;

/// Which top-level nav entry is active.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    None,
    Home,
    Skills,
    Builder,
    Diff,
    Search,
    Raw,
    Versions,
}

/// Runs before first paint so the saved theme never flashes.
const THEME_BOOT: &str = r#"try{var t=localStorage.getItem("theme");if(t==="light"||t==="dark")document.documentElement.dataset.theme=t}catch(e){}"#;

pub struct PageOpts<'a> {
    pub title: &'a str,
    pub nav: Nav,
    /// Extra scripts for this page (served from /assets).
    pub scripts: &'a [&'a str],
}

pub fn page(site: Option<&Site>, title: &str, nav: Nav, content: Markup) -> Markup {
    page_with(
        site,
        PageOpts {
            title,
            nav,
            scripts: &[],
        },
        content,
    )
}

pub fn page_with(site: Option<&Site>, opts: PageOpts, content: Markup) -> Markup {
    let link = |path: &str| site.map_or_else(|| path.to_owned(), |s| s.link(path));
    let item = |nav: Nav, href: String, label: &str| {
        html! {
            a href=(href) aria-current=[(opts.nav == nav).then_some("page")] { (label) }
        }
    };
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="color-scheme" content="light dark";
                title { (opts.title) " · datamine" }
                script { (PreEscaped(THEME_BOOT)) }
                link rel="stylesheet" href=(assets::url("app.css"));
                script src=(assets::url("app.js")) defer {}
                @for s in opts.scripts {
                    script src=(assets::url(s)) defer {}
                }
            }
            body {
                a.skip href="#main" { "Skip to content" }
                header.site {
                    div.bar {
                        a.brand href=(link("/")) {
                            span.logo aria-hidden="true" {}
                            span { "datamine" }
                        }
                        @if let Some(site) = site { (version_switcher(site)) }
                        button.icon-button.theme-toggle type="button" data-theme-toggle
                            aria-label="Switch color theme" title="Switch color theme" hidden {
                            span.theme-icon aria-hidden="true" {}
                            span.theme-label.sr-only { "Theme" }
                        }
                    }
                    nav.tabs aria-label="Main" {
                        div.tabs-inner {
                            (item(Nav::Home, link("/"), "Home"))
                            (item(Nav::Skills, link("/skills"), "Skills"))
                            (item(Nav::Builder, link("/tools/skill-builder"), "Skill builder"))
                            (item(Nav::Diff, "/diff".into(), "Patch diff"))
                            (item(Nav::Search, link("/search"), "Search"))
                            @if let Some(site) = site {
                                (item(Nav::Raw, site.raw("browse", ""), "Raw data"))
                            }
                            (item(Nav::Versions, "/versions".into(), "Versions"))
                        }
                    }
                }
                @if let Some(site) = site {
                    @if site.pinned {
                        div.pin-banner role="status" {
                            div.bar {
                                span { "Viewing " strong { (site.version.label) } ", not the newest version." }
                                a href=(site.path) { "Go to newest" }
                            }
                        }
                    }
                }
                main #main { (content) }
                footer.site {
                    div.bar.muted.small {
                        span { "Open source datamining tool. Not affiliated with the game's publisher." }
                    }
                }
            }
        }
    }
}

fn version_switcher(site: &Site) -> Markup {
    html! {
        form.version-switcher method="get" action=(site.path) {
            label {
                span.sr-only { "Version" }
                select name="v" data-autosubmit {
                    @for v in site.versions.iter().rev() {
                        option value=(v.label) selected[v.id == site.version.id] {
                            (v.label)
                            @if site.versions.last().is_some_and(|l| l.id == v.id) { " (newest)" }
                        }
                    }
                }
            }
            noscript { button type="submit" { "Go" } }
        }
    }
}

/// `a / b / c` breadcrumb; `parts` are `(label, href)`, the last is current.
pub fn breadcrumbs(parts: &[(String, String)]) -> Markup {
    html! {
        nav.crumbs aria-label="Breadcrumb" {
            ol {
                @for (i, (label, href)) in parts.iter().enumerate() {
                    li {
                        @if i + 1 == parts.len() {
                            span aria-current="page" { (label) }
                        } @else {
                            a href=(href) { (label) }
                        }
                    }
                }
            }
        }
    }
}

pub fn badge(text: &str, tone: &str) -> Markup {
    html! { span class={ "badge " (tone) } { (text) } }
}

/// `/v/<label>/<route>/<path>`: raw-data URL of a version.
pub fn raw_url(version: &Version, route: &str, wz_path: &str) -> String {
    format!(
        "/v/{}/{route}/{}",
        encode_path(&version.label),
        encode_path(wz_path)
    )
}

/// A canvas from a version's snapshot, or a neutral placeholder.
pub fn icon(version: &Version, wz_path: Option<&str>, class: &str) -> Markup {
    match wz_path {
        Some(p) => html! {
            img class={ "icon " (class) } src=(raw_url(version, "image", p)) alt="" decoding="async";
        },
        None => html! { span class={ "icon placeholder " (class) } aria-hidden="true" {} },
    }
}

/// In-game text: `\n` line breaks and `#c...#` highlights
/// (`#b`/`#r`/`#e` also start a highlight, `#k`/`#n` end it).
pub fn game_text(s: &str) -> Markup {
    let s = s.replace("\\r", "").replace('\u{FFFD}', " ");
    let mut out = String::new();
    let mut open = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'n') => {
                chars.next();
                out.push_str("<br>");
            }
            '#' => {
                let code = chars.peek().copied();
                if matches!(code, Some('c' | 'b' | 'r' | 'e' | 'k' | 'n')) {
                    chars.next();
                }
                let starts = matches!(code, Some('c' | 'b' | 'r' | 'e'));
                if open {
                    out.push_str("</mark>");
                    open = false;
                }
                if starts {
                    out.push_str("<mark>");
                    open = true;
                }
            }
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    if open {
        out.push_str("</mark>");
    }
    PreEscaped(out)
}

/// First sentence of a description without the `[Master Level: N]` prefix.
pub fn summary(desc: &str) -> String {
    let d = desc.trim_start();
    let d = match d.strip_prefix('[').and_then(|r| r.split_once(']')) {
        Some((_, rest)) => rest,
        None => d,
    };
    let d = d.trim_start_matches("\\n").trim();
    let first = d.split("\\n").next().unwrap_or(d);
    first.replace('#', "").trim().to_owned()
}

/// Compact, readable rendering of a JSON value (strings unquoted).
pub fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) if a.len() == 2 => format!("({}, {})", a[0], a[1]),
        other => other.to_string(),
    }
}

/// Inline display of an optional JSON payload as a key/value list.
pub fn data_list(v: Option<&Value>) -> Markup {
    match v {
        Some(Value::Object(map)) if !map.is_empty() => html! {
            dl.kv {
                @for (k, val) in map {
                    dt { (k) }
                    dd {
                        @match val {
                            Value::Object(o) => span.muted { (o.len()) " entries" },
                            other => (value_text(other)),
                        }
                    }
                }
            }
        },
        Some(other) => html! { code { (value_text(other)) } },
        None => html! { span.muted { "no data" } },
    }
}

/// Characters escaped inside one URL path segment.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

/// Percent-encode a `/`-separated path, keeping the separators.
pub fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|seg| utf8_percent_encode(seg, SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn encode_query(value: &str) -> String {
    utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string()
}

pub fn short_time(t: &str) -> String {
    t.get(..16).unwrap_or(t).replace('T', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_text_formats_codes_and_escapes() {
        let html = game_text(r"Hit #cfast#.\n<b>").into_string();
        assert_eq!(html, "Hit <mark>fast</mark>.<br>&lt;b&gt;");
    }

    #[test]
    fn summary_strips_master_level() {
        assert_eq!(
            summary(r"[Master Level: 20]\nUse MP to hit.\nMore"),
            "Use MP to hit."
        );
    }
}
