//! Comparing versions: changesets between two versions and the history of
//! individual records across all versions.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use crate::db::{RecordFilter, RecordPair, StoredRecord, Version};
use crate::facets::{self, FacetFilter, Facets};
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub kind: String,
    pub key: String,
    pub status: Status,
    pub facets: Facets,
    pub old: Option<Value>,
    pub new: Option<Value>,
    /// Per-field changes, when both sides have object payloads.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FieldChange {
    /// `/`-separated path inside the payload.
    pub path: String,
    pub old: Option<Value>,
    pub new: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Changeset {
    pub from: Version,
    pub to: Version,
    pub changes: Vec<Change>,
}

impl Change {
    /// A modification where only wording changed (every changed field is
    /// text on both sides), e.g. a reworded description.
    pub fn is_text_only(&self) -> bool {
        let is_text = |v: &Option<Value>| matches!(v, None | Some(Value::String(_)));
        self.status == Status::Modified
            && !self.fields.is_empty()
            && self
                .fields
                .iter()
                .all(|f| is_text(&f.old) && is_text(&f.new))
    }

    /// Display name from the payload (`name`, `mapName`, ...), if any.
    pub fn title(&self) -> Option<&str> {
        let data = self.new.as_ref().or(self.old.as_ref())?;
        ["name", "mapName", "streetName", "bookName"]
            .iter()
            .find_map(|k| data.get(k).and_then(Value::as_str))
    }
}

impl Changeset {
    /// Drop modifications where only wording changed.
    pub fn hide_text_only(&mut self) {
        self.changes.retain(|c| !c.is_text_only());
    }

    /// Keep only changes matching `filter`.
    pub fn retain(&mut self, filter: &FacetFilter) {
        self.changes.retain(|c| filter.matches(&c.facets));
    }

    /// Changes grouped by kind, in kind order.
    pub fn by_kind(&self) -> Vec<(&str, Vec<&Change>)> {
        let mut out: Vec<(&str, Vec<&Change>)> = Vec::new();
        for c in &self.changes {
            match out.last_mut() {
                Some((k, list)) if *k == c.kind => list.push(c),
                _ => out.push((&c.kind, vec![c])),
            }
        }
        out
    }

    /// `(kind, added, removed, modified)` counts, sorted by kind.
    pub fn summary(&self) -> Vec<(String, usize, usize, usize)> {
        let mut counts: BTreeMap<&str, [usize; 3]> = BTreeMap::new();
        for c in &self.changes {
            counts.entry(&c.kind).or_default()[c.status as usize] += 1;
        }
        counts
            .into_iter()
            .map(|(k, [a, r, m])| (k.to_owned(), a, r, m))
            .collect()
    }
}

pub fn changeset(
    store: &Store,
    from: &Version,
    to: &Version,
    filter: &RecordFilter,
) -> Result<Changeset> {
    let changes = store
        .db()
        .diff(from.id, to.id, filter)?
        .into_iter()
        .map(change_from_pair)
        .collect();
    Ok(Changeset {
        from: from.clone(),
        to: to.clone(),
        changes,
    })
}

fn change_from_pair(p: RecordPair) -> Change {
    let status = match (&p.old_hash, &p.new_hash) {
        (None, _) => Status::Added,
        (_, None) => Status::Removed,
        _ => Status::Modified,
    };
    let fields = match (status, &p.old_data, &p.new_data) {
        (Status::Modified, Some(old), Some(new)) => field_changes(old, new),
        _ => Vec::new(),
    };
    let facets = facets::of(&p.kind, &p.key, p.new_data.as_ref().or(p.old_data.as_ref()));
    Change {
        kind: p.kind,
        key: p.key,
        status,
        facets,
        old: p.old_data,
        new: p.new_data,
        fields,
    }
}

/// Leaf-level differences between two JSON values. Objects are compared
/// key by key; anything else is compared as a whole.
pub fn field_changes(old: &Value, new: &Value) -> Vec<FieldChange> {
    let mut out = Vec::new();
    walk(String::new(), Some(old), Some(new), &mut out);
    out
}

fn walk(path: String, old: Option<&Value>, new: Option<&Value>, out: &mut Vec<FieldChange>) {
    match (old, new) {
        (Some(Value::Object(a)), Some(Value::Object(b))) => {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for k in keys {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}/{k}")
                };
                walk(p, a.get(k), b.get(k), out);
            }
        }
        (a, b) if a != b => out.push(FieldChange {
            path,
            old: a.cloned(),
            new: b.cloned(),
        }),
        _ => {}
    }
}

/// One point in a record's history: a version where it was added, changed
/// or removed.
#[derive(Debug, Clone, Serialize)]
pub struct HistoryEvent {
    pub version: String,
    pub status: Status,
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordHistory {
    pub kind: String,
    pub key: String,
    pub events: Vec<HistoryEvent>,
}

/// History of every record matching `key` (exactly or as a `/<key>`
/// suffix), collapsed to the versions where something changed.
pub fn history(store: &Store, key: &str, filter: &RecordFilter) -> Result<Vec<RecordHistory>> {
    let versions = store.db().versions()?;
    let rows = store.db().history(key, filter)?;

    let mut grouped: BTreeMap<(String, String), BTreeMap<i64, StoredRecord>> = BTreeMap::new();
    for row in rows {
        grouped
            .entry((row.kind.clone(), row.key.clone()))
            .or_default()
            .insert(row.version_id, row);
    }

    grouped
        .into_iter()
        .map(|((kind, key), states)| {
            // Versions that never produced this kind (e.g. not extracted yet)
            // say nothing about the record, so leave them out.
            let covered = store.db().versions_with_kind(&kind)?;
            let relevant: Vec<_> = versions
                .iter()
                .filter(|v| covered.contains(&v.id))
                .cloned()
                .collect();
            Ok(RecordHistory {
                events: events(&relevant, &states),
                kind,
                key,
            })
        })
        .collect()
}

fn events(versions: &[Version], states: &BTreeMap<i64, StoredRecord>) -> Vec<HistoryEvent> {
    let mut out = Vec::new();
    let mut prev: Option<&StoredRecord> = None;
    for v in versions {
        let cur = states.get(&v.id);
        match (prev, cur) {
            (None, Some(c)) => out.push(HistoryEvent {
                version: v.label.clone(),
                status: Status::Added,
                data: c.data.clone(),
                fields: Vec::new(),
            }),
            (Some(_), None) => out.push(HistoryEvent {
                version: v.label.clone(),
                status: Status::Removed,
                data: None,
                fields: Vec::new(),
            }),
            (Some(p), Some(c)) if p.hash != c.hash => out.push(HistoryEvent {
                version: v.label.clone(),
                status: Status::Modified,
                data: c.data.clone(),
                fields: match (&p.data, &c.data) {
                    (Some(a), Some(b)) => field_changes(a, b),
                    _ => Vec::new(),
                },
            }),
            _ => {}
        }
        prev = cur;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_only_changes() {
        let change = |old: Value, new: Value| Change {
            kind: "skill".into(),
            key: "1".into(),
            status: Status::Modified,
            facets: crate::facets::of("skill", "1", None),
            fields: field_changes(&old, &new),
            old: Some(old),
            new: Some(new),
        };
        assert!(change(json!({"desc": "a", "n": 1}), json!({"desc": "b", "n": 1})).is_text_only());
        assert!(!change(json!({"desc": "a", "n": 1}), json!({"desc": "b", "n": 2})).is_text_only());
        assert!(!change(json!({"n": 1}), json!({"n": "1"})).is_text_only());
    }

    #[test]
    fn field_changes_reports_leaves() {
        let old = json!({"name": "Sword", "desc": "old", "info": {"lv": 10}});
        let new = json!({"name": "Sword", "info": {"lv": 20}, "extra": 1});
        let fields = field_changes(&old, &new);
        let paths: Vec<_> = fields.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["desc", "extra", "info/lv"]);
        assert_eq!(fields[2].new, Some(json!(20)));
    }
}
