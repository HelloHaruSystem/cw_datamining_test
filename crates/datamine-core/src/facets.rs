//! Facets classify records for filtering ("show me Warrior skill changes").
//!
//! They are derived from a record's kind, key and data at query time, so
//! adding a facet never requires re-extracting anything.

use serde::Serialize;
use serde_json::Value;

use crate::jobs::{self, Branch, JobId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Skills,
    Equipment,
    Items,
    Monsters,
    Npcs,
    Maps,
    Quests,
    Other,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::Skills,
        Category::Equipment,
        Category::Items,
        Category::Monsters,
        Category::Npcs,
        Category::Maps,
        Category::Quests,
        Category::Other,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Category::Skills => "skills",
            Category::Equipment => "equipment",
            Category::Items => "items",
            Category::Monsters => "monsters",
            Category::Npcs => "npcs",
            Category::Maps => "maps",
            Category::Quests => "quests",
            Category::Other => "other",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Category::Skills => "Skills",
            Category::Equipment => "Equipment",
            Category::Items => "Items",
            Category::Monsters => "Monsters",
            Category::Npcs => "NPCs",
            Category::Maps => "Maps",
            Category::Quests => "Quests",
            Category::Other => "Other",
        }
    }

    pub fn from_slug(s: &str) -> Option<Category> {
        Category::ALL
            .into_iter()
            .find(|c| c.slug().eq_ignore_ascii_case(s))
    }
}

/// Whether a record is extracted, readable data or a raw file/`.img` hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Detail {
    Data,
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Facets {
    pub category: Category,
    pub job: Option<JobId>,
    pub detail: Detail,
}

pub fn of(kind: &str, key: &str, data: Option<&Value>) -> Facets {
    let detail = if matches!(kind, "img" | "file") {
        Detail::Raw
    } else {
        Detail::Data
    };
    let (category, job) = match kind {
        "skill" => (Category::Skills, skill_job(key, data)),
        "string/Skill" => (Category::Skills, skill_job(key, None)),
        "string/Eqp" => (Category::Equipment, None),
        "string/Consume" | "string/Etc" | "string/Ins" | "string/Cash" | "string/Pet" => {
            (Category::Items, None)
        }
        "string/Mob" | "string/MobSkill" => (Category::Monsters, None),
        "string/Npc" => (Category::Npcs, None),
        "string/Map" => (Category::Maps, None),
        "img" | "file" => raw_facets(key),
        _ => (Category::Other, None),
    };
    Facets {
        category,
        job,
        detail,
    }
}

fn skill_job(key: &str, data: Option<&Value>) -> Option<JobId> {
    data.and_then(|d| d.get("job"))
        .and_then(Value::as_u64)
        .map(|j| j as JobId)
        .or_else(|| key.parse::<u32>().ok().map(jobs::of_skill))
}

/// Facets from a WZ path such as `Skill/110.img` or `Data/Mob/Mob_000.wz`.
fn raw_facets(path: &str) -> (Category, Option<JobId>) {
    let path = path.strip_prefix("Data/").unwrap_or(path);
    let top = path.split('/').next().unwrap_or_default();
    let category = match top {
        "Skill" => Category::Skills,
        "Character" => Category::Equipment,
        "Item" => Category::Items,
        "Mob" => Category::Monsters,
        "Npc" => Category::Npcs,
        "Map" => Category::Maps,
        "Quest" => Category::Quests,
        _ => Category::Other,
    };
    let job = (category == Category::Skills)
        .then(|| {
            path.strip_prefix("Skill/")?
                .strip_suffix(".img")?
                .parse()
                .ok()
        })
        .flatten();
    (category, job)
}

/// Which records to keep. Empty lists mean "no restriction".
#[derive(Debug, Clone, Default)]
pub struct FacetFilter {
    pub categories: Vec<Category>,
    /// Matches the job itself; see [`FacetFilter::branches`] for whole lines.
    pub jobs: Vec<JobId>,
    pub branches: Vec<Branch>,
    /// Job advancements (0 = beginner, 1..=4), across all branches.
    pub advancements: Vec<u8>,
    /// Hide raw file/`.img` records.
    pub data_only: bool,
}

impl FacetFilter {
    pub fn matches(&self, f: &Facets) -> bool {
        (self.categories.is_empty() || self.categories.contains(&f.category))
            && (self.jobs.is_empty() || f.job.is_some_and(|j| self.jobs.contains(&j)))
            && (self.branches.is_empty()
                || f.job
                    .and_then(jobs::branch)
                    .is_some_and(|b| self.branches.contains(&b)))
            && (self.advancements.is_empty()
                || f.job
                    .and_then(jobs::advancement)
                    .is_some_and(|a| self.advancements.contains(&a)))
            && (!self.data_only || f.detail == Detail::Data)
    }

    /// Add an advancement selector: `beginner`, `1st`..`4th` or `0`..`4`.
    pub fn add_advancement(&mut self, selector: &str) -> Result<(), String> {
        let a = jobs::parse_advancement(selector).ok_or_else(|| {
            format!("unknown advancement {selector:?}; use beginner, 1st, 2nd, 3rd or 4th")
        })?;
        self.advancements.push(a);
        Ok(())
    }

    /// Add a job selector: a branch (`warrior`), a job name (`fighter`)
    /// or a job id (`110`).
    pub fn add_job(&mut self, selector: &str) -> Result<(), String> {
        if let Some(b) = Branch::from_slug(selector) {
            self.branches.push(b);
        } else if let Ok(id) = selector.parse::<JobId>() {
            self.jobs.push(id);
        } else if let Some(j) = jobs::JOBS
            .iter()
            .find(|j| j.name.eq_ignore_ascii_case(selector))
        {
            self.jobs.push(j.id);
        } else {
            return Err(format!(
                "unknown job {selector:?}: use a branch, job name or id"
            ));
        }
        Ok(())
    }

    pub fn add_category(&mut self, slug: &str) -> Result<(), String> {
        let c = Category::from_slug(slug).ok_or_else(|| {
            let all: Vec<_> = Category::ALL.iter().map(|c| c.slug()).collect();
            format!("unknown category {slug:?}; use one of {}", all.join(", "))
        })?;
        self.categories.push(c);
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.categories.is_empty()
            && self.jobs.is_empty()
            && self.branches.is_empty()
            && self.advancements.is_empty()
            && !self.data_only
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_records() {
        let f = of("skill", "1101006", Some(&json!({"job": 110})));
        assert_eq!(
            (f.category, f.job, f.detail),
            (Category::Skills, Some(110), Detail::Data)
        );
        assert_eq!(of("string/Skill", "0001000", None).job, Some(0));
        let raw = of("img", "Skill/111.img", None);
        assert_eq!(
            (raw.category, raw.job, raw.detail),
            (Category::Skills, Some(111), Detail::Raw)
        );
        assert_eq!(
            of("file", "Data/Mob/Mob_000.wz", None).category,
            Category::Monsters
        );
    }

    #[test]
    fn filter_by_advancement_and_branch() {
        let mut filter = FacetFilter::default();
        filter.add_advancement("2nd").unwrap();
        filter.add_job("warrior").unwrap();
        assert!(filter.matches(&of("skill", "1101006", None))); // Fighter
        assert!(!filter.matches(&of("skill", "1001003", None))); // Warrior, 1st
        assert!(!filter.matches(&of("skill", "2101001", None))); // F/P Wizard
        assert!(filter.add_advancement("5th").is_err());
    }

    #[test]
    fn filter_by_branch() {
        let filter = FacetFilter {
            branches: vec![Branch::Warrior],
            ..Default::default()
        };
        assert!(filter.matches(&of("skill", "1101006", None)));
        assert!(!filter.matches(&of("skill", "2001002", None)));
        assert!(!filter.matches(&of("string/Eqp", "x", None)));
    }
}
