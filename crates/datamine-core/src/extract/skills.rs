//! Skills, combined from `Skill/<job>.img` and `String/Skill.img`.
//!
//! One record of kind `skill` per skill, keyed by its id as stored in the
//! WZ (`"1001002"`, `"0001000"`). The payload is shaped for reading and for
//! field-level diffs, e.g. `levels/12/damage`:
//!
//! ```json
//! { "job": 100, "name": "Power Strike", "desc": "...", "max_level": 20,
//!   "req": { "1000000": 3 }, "invisible": true,
//!   "levels": { "1": { "text": "MP -3; Damage 165%", "damage": 165 } } }
//! ```

use std::collections::HashMap;

use anyhow::Result;
use serde_json::{Map, Value, json};
use wz_reader::WzNodeArc;

use super::Extractor;
use crate::jobs::JobId;
use crate::record::Record;
use crate::wz::{self, JsonOptions, WzTree};

pub const KIND: &str = "skill";

pub struct Skills;

impl Extractor for Skills {
    fn name(&self) -> &'static str {
        "skills"
    }

    fn description(&self) -> &'static str {
        "skills with job, prerequisites, max level and per-level stats (kind `skill`)"
    }

    fn extract(&self, tree: &WzTree) -> Result<Vec<Record>> {
        let strings = skill_strings(tree)?;
        let mut out = Vec::new();
        for (path, img) in tree.images()? {
            let Some(job) = job_image(&path) else {
                continue;
            };
            wz::parse(&img)?;
            if let Ok(skills) = tree.get(&format!("{path}/skill")) {
                for (id, node) in wz::children(&skills) {
                    let data = skill_data(job, &node, strings.get(&id));
                    out.push(Record::from_data(KIND, id, data));
                }
            }
            wz::unparse(&img);
        }
        Ok(out)
    }
}

/// `Skill/110.img` -> `Some(110)`.
fn job_image(path: &str) -> Option<JobId> {
    path.strip_prefix("Skill/")?
        .strip_suffix(".img")?
        .parse()
        .ok()
}

/// `String/Skill.img` as id -> {name, desc, h1, ...}.
fn skill_strings(tree: &WzTree) -> Result<HashMap<String, Map<String, Value>>> {
    let img = tree.get("String/Skill.img")?;
    let out = wz::children(&img)
        .into_iter()
        .filter_map(
            |(id, node)| match wz::node_to_json(&node, JsonOptions::default()) {
                Value::Object(map) => Some((id, map)),
                _ => None,
            },
        )
        .collect();
    wz::unparse(&img);
    Ok(out)
}

fn skill_data(job: JobId, node: &WzNodeArc, strings: Option<&Map<String, Value>>) -> Value {
    let get = |name: &str| {
        wz::children(node)
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, child)| wz::node_to_json(&child, JsonOptions::default()))
    };
    let text = |key: &str| strings.and_then(|s| s.get(key)).and_then(Value::as_str);

    let mut levels = Map::new();
    if let Some(Value::Object(raw)) = get("level") {
        for (lv, stats) in raw {
            let Value::Object(stats) = stats else {
                continue;
            };
            let mut entry: Map<String, Value> = stats
                .into_iter()
                .filter(|(k, v)| k != "hs" && !v.is_object())
                .collect();
            let text_key = entry
                .get("hs")
                .and_then(Value::as_str)
                .map_or_else(|| format!("h{lv}"), str::to_owned);
            if let Some(t) = text(&text_key) {
                entry.insert("text".into(), t.into());
            }
            levels.insert(lv, Value::Object(entry));
        }
    }

    let mut data = Map::new();
    data.insert("job".into(), job.into());
    if let Some(name) = text("name") {
        data.insert("name".into(), name.into());
    }
    if let Some(desc) = text("desc") {
        data.insert("desc".into(), desc.into());
    }
    let common = get("common");
    let max_level = if levels.is_empty() {
        common
            .as_ref()
            .and_then(|c| c.get("maxLevel"))
            .and_then(|m| {
                m.as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| m.as_u64())
            })
            .unwrap_or(0)
    } else {
        levels.len() as u64
    };
    data.insert("max_level".into(), max_level.into());
    if let Some(Value::Object(req)) = get("req") {
        data.insert("req".into(), Value::Object(req));
    }
    if get("invisible").is_some_and(|v| v != json!(0)) {
        data.insert("invisible".into(), true.into());
    }
    if let Some(elem) = get("elemAttr") {
        data.insert("element".into(), elem);
    }
    if let Some(common) = common {
        data.insert("common".into(), common);
    }
    data.insert("icon".into(), get("icon").is_some().into());
    if !levels.is_empty() {
        data.insert("levels".into(), Value::Object(levels));
    }
    Value::Object(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_images() {
        assert_eq!(job_image("Skill/110.img"), Some(110));
        assert_eq!(job_image("Skill/000.img"), Some(0));
        assert_eq!(job_image("Skill/Attacktype.img"), None);
        assert_eq!(job_image("String/110.img"), None);
    }
}
