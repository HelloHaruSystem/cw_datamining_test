//! Page shell and small reusable pieces of markup.

use maud::{DOCTYPE, Markup, PreEscaped, html};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde_json::Value;

/// Which top-level nav entry is active.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    None,
    Versions,
    Search,
    Diff,
}

/// Runs before first paint so the saved theme never flashes.
const THEME_BOOT: &str = r#"try{var t=localStorage.getItem("theme");if(t==="light"||t==="dark")document.documentElement.dataset.theme=t}catch(e){}"#;

pub fn page(title: &str, active: Nav, content: Markup) -> Markup {
    let item = |nav: Nav, href: &str, label: &str| {
        html! {
            a href=(href) aria-current=[(active == nav).then_some("page")] { (label) }
        }
    };
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="color-scheme" content="light dark";
                title { (title) " · datamine" }
                script { (PreEscaped(THEME_BOOT)) }
                link rel="stylesheet" href="/assets/app.css";
                script src="/assets/app.js" defer {}
            }
            body {
                a.skip href="#main" { "Skip to content" }
                header.site {
                    div.bar {
                        a.brand href="/" { "datamine" }
                        nav aria-label="Main" {
                            (item(Nav::Versions, "/", "Versions"))
                            (item(Nav::Search, "/search", "Search"))
                            (item(Nav::Diff, "/diff", "Diff"))
                        }
                        button.theme-toggle type="button" data-theme-toggle
                            aria-label="Switch color theme" title="Switch color theme" hidden {
                            span.theme-label { "Theme" }
                        }
                    }
                }
                main #main { (content) }
            }
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
                    dd { (value_text(val)) }
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
