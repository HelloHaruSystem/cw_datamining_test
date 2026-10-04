//! Human-readable output. Pure formatting: takes core types, returns or
//! prints text.

use std::fmt::Write;

use datamine_core::catalog::Skill;
use datamine_core::db::{Extraction, StoredRecord, Version, VersionId};
use datamine_core::diff::{Changeset, RecordHistory, Status};
use datamine_core::extract::ExtractSummary;
use datamine_core::jobs::{self, JobId};
use datamine_core::sp::Evaluation;
use datamine_core::textdiff;
use serde_json::Value;

pub fn versions(versions: &[Version], baseline: Option<VersionId>) {
    if versions.is_empty() {
        println!("no versions yet; run `datamine import`");
        return;
    }
    println!(
        "{:<4} {:<24} {:<11} {:<26} build",
        "id", "label", "channel", "imported"
    );
    for v in versions {
        let mark = if Some(v.id) == baseline {
            " (baseline)"
        } else {
            ""
        };
        println!(
            "{:<4} {:<24} {:<11} {:<26} {}{mark}",
            format!("#{}", v.id),
            v.label,
            v.channel,
            short_time(&v.imported_at),
            v.build_time.as_deref().map(short_time).unwrap_or("-"),
        );
    }
}

pub fn info(v: &Version, extractions: &[Extraction], kinds: &[(String, i64)]) {
    println!("{} (#{}, {})", v.label, v.id, v.channel);
    println!("  imported   {}", v.imported_at);
    if let Some(b) = &v.build_time {
        println!("  build      {b}");
    }
    if let Some(h) = &v.manifest_hash {
        println!("  manifest   {h}");
    }
    if let Some(n) = &v.note {
        println!("  note       {n}");
    }
    println!("\nextractions:");
    for e in extractions {
        println!(
            "  {:<10} {:>8} records  {}",
            e.extractor,
            e.record_count,
            short_time(&e.finished_at)
        );
    }
    println!("\nrecords by kind:");
    for (kind, count) in kinds {
        println!("  {kind:<24} {count:>8}");
    }
}

pub fn extract_summaries(summaries: &[ExtractSummary]) {
    for s in summaries {
        println!(
            "extracted {:<10} {:>8} records in {:.1}s",
            s.extractor, s.records, s.seconds
        );
    }
}

pub fn changeset_text(cs: &Changeset, limit: usize) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{} -> {}", cs.from.label, cs.to.label);
    if cs.changes.is_empty() {
        out.push_str("no changes\n");
        return out;
    }
    let _ = writeln!(
        out,
        "\n{:<28} {:>7} {:>7} {:>8}",
        "kind", "added", "removed", "modified"
    );
    for (kind, a, r, m) in cs.summary() {
        let _ = writeln!(out, "{kind:<28} {a:>7} {r:>7} {m:>8}");
    }
    for (kind, changes) in cs.by_kind() {
        let _ = writeln!(out, "\n== {kind} ==");
        for c in changes.iter().take(limit_or_all(limit)) {
            let sigil = match c.status {
                Status::Added => '+',
                Status::Removed => '-',
                Status::Modified => '~',
            };
            let _ = write!(out, "{sigil} {}", c.key);
            match c.status {
                Status::Added => {
                    let _ = writeln!(out, "  {}", opt_preview(&c.new, 100));
                }
                Status::Removed => {
                    let _ = writeln!(out, "  {}", opt_preview(&c.old, 100));
                }
                Status::Modified => {
                    out.push('\n');
                    for f in &c.fields {
                        let _ = writeln!(out, "    {}: {}", f.path, field_text(&f.old, &f.new));
                    }
                }
            }
        }
        more(&mut out, changes.len(), limit);
    }
    out
}

pub fn changeset_markdown(cs: &Changeset, limit: usize) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Changes: {} → {}\n", cs.from.label, cs.to.label);
    if cs.changes.is_empty() {
        out.push_str("No changes.\n");
        return out;
    }
    out.push_str("| Kind | Added | Removed | Modified |\n|---|---:|---:|---:|\n");
    for (kind, a, r, m) in cs.summary() {
        let _ = writeln!(out, "| `{kind}` | {a} | {r} | {m} |");
    }
    for (kind, changes) in cs.by_kind() {
        let _ = writeln!(out, "\n## `{kind}`\n");
        for c in changes.iter().take(limit_or_all(limit)) {
            match c.status {
                Status::Added => {
                    let _ = writeln!(out, "- **added** `{}` {}", c.key, md_value(&c.new));
                }
                Status::Removed => {
                    let _ = writeln!(out, "- **removed** `{}` {}", c.key, md_value(&c.old));
                }
                Status::Modified => {
                    let _ = writeln!(out, "- **changed** `{}`", c.key);
                    for f in &c.fields {
                        let _ = writeln!(
                            out,
                            "  - `{}`: {} → {}",
                            f.path,
                            md_value(&f.old),
                            md_value(&f.new)
                        );
                    }
                }
            }
        }
        more(&mut out, changes.len(), limit);
    }
    out
}

pub fn history_text(hist: &[RecordHistory]) -> String {
    let mut out = String::new();
    for h in hist {
        let _ = writeln!(out, "{} {}", h.kind, h.key);
        for e in &h.events {
            let status = match e.status {
                Status::Added => "added",
                Status::Removed => "removed",
                Status::Modified => "changed",
            };
            let _ = write!(out, "  {:<24} {status:<8}", e.version);
            if e.fields.is_empty() {
                let _ = writeln!(out, " {}", opt_preview(&e.data, 90));
            } else {
                out.push('\n');
                for f in &e.fields {
                    let _ = writeln!(
                        out,
                        "      {}: {} -> {}",
                        f.path,
                        opt_preview(&f.old, 50),
                        opt_preview(&f.new, 50)
                    );
                }
            }
        }
    }
    out
}

pub fn skill(s: &Skill) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} ({})  {}  max level {}",
        s.label(),
        s.id,
        jobs::name(s.job),
        s.max_level
    );
    if !s.req.is_empty() {
        let req: Vec<_> = s
            .req
            .iter()
            .map(|(id, lv)| format!("{id} at level {lv}"))
            .collect();
        let _ = writeln!(out, "requires {}", req.join(", "));
    }
    if s.invisible {
        out.push_str("hidden skill\n");
    }
    if let Some(d) = &s.desc {
        let _ = writeln!(out, "\n{}", d.replace("\\n", "\n"));
    }
    out.push('\n');
    for (lv, stats) in &s.levels {
        let text = stats
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let nums: Vec<_> = stats
            .iter()
            .filter(|(k, _)| *k != "text")
            .map(|(k, v)| format!("{k}={}", preview(v, 20)))
            .collect();
        let _ = writeln!(out, "{lv:>3}  {text}");
        if !nums.is_empty() {
            let _ = writeln!(out, "     {}", nums.join(" "));
        }
    }
    out
}

pub fn sp(eval: &Evaluation, target: JobId, level: u32) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{} at level {level}", jobs::name(target));
    for p in &eval.pools {
        let _ = writeln!(
            out,
            "  {:<26} {:>3} / {:>3} SP  ({} left)",
            p.job_name,
            p.spent,
            p.total,
            p.total as i64 - p.spent as i64
        );
    }
    if eval.problems.is_empty() {
        out.push_str("build is valid\n");
    } else {
        out.push_str("problems:\n");
        for p in &eval.problems {
            let _ = writeln!(out, "  - {p}");
        }
    }
    out
}

pub fn search(hits: &[StoredRecord], limit: usize) {
    for h in hits {
        println!("{:<20} {:<32} {}", h.kind, h.key, opt_preview(&h.data, 80));
    }
    if hits.len() == limit {
        println!("(showing first {limit}; use --limit for more)");
    }
}

/// One-line JSON preview, truncated to `max` characters.
pub fn preview(v: &Value, max: usize) -> String {
    let s = match v {
        Value::String(s) => format!("{s:?}"),
        other => other.to_string(),
    };
    truncate(&s, max)
}

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    format!("{v:.1} {}", UNITS[unit])
}

/// `old -> new`, or for edited text a word diff around the first change:
/// `…from items [-and skills -]is boosted.`
fn field_text(old: &Option<Value>, new: &Option<Value>) -> String {
    if let (Some(Value::String(a)), Some(Value::String(b))) = (old, new) {
        let segments = textdiff::diff_words(a, b);
        if textdiff::similarity(&segments) >= 0.3 {
            let inline = textdiff::inline(a, b);
            let first = inline
                .find("[-")
                .into_iter()
                .chain(inline.find("{+"))
                .min()
                .unwrap_or(0);
            let start = inline[..first]
                .char_indices()
                .rev()
                .nth(40)
                .map_or(0, |(i, _)| i);
            let prefix = if start > 0 { "…" } else { "" };
            return format!("{prefix}{}", truncate(&inline[start..], 140));
        }
    }
    format!("{} -> {}", opt_preview(old, 60), opt_preview(new, 60))
}

fn limit_or_all(limit: usize) -> usize {
    if limit == 0 { usize::MAX } else { limit }
}

fn more(out: &mut String, total: usize, limit: usize) {
    if limit != 0 && total > limit {
        let _ = writeln!(out, "  … {} more (use --limit 0 for all)", total - limit);
    }
}

fn opt_preview(v: &Option<Value>, max: usize) -> String {
    v.as_ref().map_or_else(|| "∅".into(), |v| preview(v, max))
}

fn md_value(v: &Option<Value>) -> String {
    format!("`{}`", opt_preview(v, 120).replace('`', "'"))
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', "\\n").replace('\r', "");
    if s.chars().count() <= max {
        return s;
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

fn short_time(t: &str) -> &str {
    t.get(..19).unwrap_or(t)
}
