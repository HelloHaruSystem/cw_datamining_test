//! Typed views over extracted records, for the database pages and the CLI.
//! Records stay the source of truth; these structs only make them
//! convenient to work with.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::db::{RecordFilter, VersionId};
use crate::extract::skills::KIND as SKILL_KIND;
use crate::jobs::{self, JobId};
use crate::store::Store;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Skill {
    pub id: String,
    pub job: JobId,
    pub name: Option<String>,
    pub desc: Option<String>,
    pub max_level: u32,
    /// Prerequisite skill id -> required level.
    pub req: BTreeMap<String, u32>,
    /// Hidden from the skill window (passive effects, internal skills).
    pub invisible: bool,
    pub has_icon: bool,
    /// Level -> stats, with the level description under `text`.
    pub levels: BTreeMap<u32, Map<String, Value>>,
}

impl Skill {
    pub fn from_record(id: &str, data: &Value) -> Self {
        let str_field = |k: &str| data.get(k).and_then(Value::as_str).map(str::to_owned);
        let levels = data
            .get("levels")
            .and_then(Value::as_object)
            .map(|lv| {
                lv.iter()
                    .filter_map(|(k, v)| Some((k.parse().ok()?, v.as_object()?.clone())))
                    .collect()
            })
            .unwrap_or_default();
        let req = data
            .get("req")
            .and_then(Value::as_object)
            .map(|r| {
                r.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_u64()? as u32)))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            id: id.to_owned(),
            job: data.get("job").and_then(Value::as_u64).map_or_else(
                || id.parse().map(jobs::of_skill).unwrap_or(0),
                |j| j as JobId,
            ),
            name: str_field("name"),
            desc: str_field("desc"),
            max_level: data.get("max_level").and_then(Value::as_u64).unwrap_or(0) as u32,
            req,
            invisible: data
                .get("invisible")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            has_icon: data.get("icon").and_then(Value::as_bool).unwrap_or(false),
            levels,
        }
    }

    /// Name, or the id when the client has no name for it.
    pub fn label(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("Skill {}", self.id))
    }

    /// Shown in the in-game skill window: visible, named, levelable.
    pub fn is_learnable(&self) -> bool {
        !self.invisible && self.name.is_some() && self.max_level > 0
    }

    /// WZ path of the icon canvas, if the skill has one.
    pub fn icon_path(&self) -> Option<String> {
        self.has_icon
            .then(|| format!("Skill/{:03}.img/skill/{}/icon", self.job, self.id))
    }
}

/// All skills of a version, sorted by job then id.
pub fn skills(store: &Store, version: VersionId) -> Result<Vec<Skill>> {
    let records = store.db().records(
        version,
        &RecordFilter {
            kind: Some(SKILL_KIND),
        },
    )?;
    let mut out: Vec<Skill> = records
        .iter()
        .filter_map(|r| Some(Skill::from_record(&r.key, r.data.as_ref()?)))
        .collect();
    out.sort_by(|a, b| a.job.cmp(&b.job).then_with(|| a.id.cmp(&b.id)));
    Ok(out)
}

pub fn skill(store: &Store, version: VersionId, id: &str) -> Result<Option<Skill>> {
    Ok(store
        .db()
        .record(version, SKILL_KIND, id)?
        .and_then(|r| Some(Skill::from_record(&r.key, r.data.as_ref()?))))
}

/// Job ids that have at least one learnable skill, sorted.
pub fn skill_jobs(skills: &[Skill]) -> Vec<JobId> {
    let mut out: Vec<JobId> = skills
        .iter()
        .filter(|s| s.is_learnable())
        .map(|s| s.job)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}
