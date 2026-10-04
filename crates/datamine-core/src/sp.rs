//! Skill point rules and build evaluation, shared by the CLI (`datamine sp`)
//! and the web skill builder.
//!
//! Classic rules: beginners get 1 SP per level for levels 2-7; each job
//! advancement grants 1 SP, then 3 SP per level. SP earned in a job can only
//! be spent on that job's skills (e.g. a level 30 warrior has a 1st job pool
//! of 1 + 3 x 20 = 61).

use std::collections::BTreeMap;

use serde::Serialize;

use crate::catalog::Skill;
use crate::jobs::{self, Branch, JobId};

#[derive(Debug, Clone, Serialize)]
pub struct SpRules {
    pub level_cap: u32,
    pub sp_per_level: u32,
    pub sp_on_advance: u32,
    /// Last level that grants beginner SP (1 per level from level 2).
    pub beginner_last_level: u32,
}

impl Default for SpRules {
    fn default() -> Self {
        Self {
            level_cap: 200,
            sp_per_level: 3,
            sp_on_advance: 1,
            beginner_last_level: 7,
        }
    }
}

impl SpRules {
    /// Level at which `job` is reached (`None` for beginner).
    pub fn advancement_level(&self, job: JobId) -> Option<u32> {
        let j = jobs::get(job)?;
        Some(match j.advancement {
            0 => return None,
            1 if j.branch == Branch::Magician => 8,
            1 => 10,
            2 => 30,
            3 => 70,
            _ => 120,
        })
    }

    /// SP available to each job on the way to `target` at `level`.
    pub fn pools(&self, target: JobId, level: u32) -> Vec<(JobId, u32)> {
        let level = level.clamp(1, self.level_cap);
        let path = jobs::path(target);
        path.iter()
            .enumerate()
            .map(|(i, &job)| {
                let total = match self.advancement_level(job) {
                    None => level.min(self.beginner_last_level).saturating_sub(1),
                    Some(start) if level < start => 0,
                    Some(start) => {
                        // Earning stops once the next job is reached.
                        let end = path
                            .get(i + 1)
                            .and_then(|&next| self.advancement_level(next))
                            .unwrap_or(self.level_cap);
                        self.sp_on_advance + self.sp_per_level * (level.min(end) - start)
                    }
                };
                (job, total)
            })
            .collect()
    }

    /// Lowest level at which `target` is reachable.
    pub fn min_level(&self, target: JobId) -> u32 {
        self.advancement_level(target).unwrap_or(1)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Pool {
    pub job: JobId,
    pub job_name: String,
    pub total: u32,
    pub spent: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evaluation {
    pub pools: Vec<Pool>,
    /// Human-readable rule violations; empty means the build is valid.
    pub problems: Vec<String>,
}

/// Check a build (skill id -> level) for `target` at `level` against the
/// skills of a version.
pub fn evaluate(
    rules: &SpRules,
    target: JobId,
    level: u32,
    build: &BTreeMap<String, u32>,
    skills: &[Skill],
) -> Evaluation {
    let by_id: BTreeMap<&str, &Skill> = skills.iter().map(|s| (s.id.as_str(), s)).collect();
    let path = jobs::path(target);
    let mut problems = Vec::new();
    let mut spent: BTreeMap<JobId, u32> = BTreeMap::new();

    if level < rules.min_level(target) {
        problems.push(format!(
            "{} needs level {}",
            jobs::name(target),
            rules.min_level(target)
        ));
    }

    for (id, &lv) in build {
        if lv == 0 {
            continue;
        }
        let Some(skill) = by_id.get(id.as_str()) else {
            problems.push(format!("unknown skill {id}"));
            continue;
        };
        if !skill.is_learnable() {
            problems.push(format!("{} cannot be learned", skill.label()));
            continue;
        }
        if !path.contains(&skill.job) {
            problems.push(format!(
                "{} is not a {} skill",
                skill.label(),
                jobs::name(target)
            ));
            continue;
        }
        if lv > skill.max_level {
            problems.push(format!(
                "{} is at {lv}, max is {}",
                skill.label(),
                skill.max_level
            ));
        }
        for (req_id, &req_lv) in &skill.req {
            let have = build.get(req_id).copied().unwrap_or(0);
            if have < req_lv {
                let req_name = by_id
                    .get(req_id.as_str())
                    .map_or(req_id.clone(), |s| s.label());
                problems.push(format!(
                    "{} needs {req_name} at level {req_lv} (has {have})",
                    skill.label()
                ));
            }
        }
        *spent.entry(skill.job).or_default() += lv;
    }

    let pools = rules
        .pools(target, level)
        .into_iter()
        .map(|(job, total)| {
            let used = spent.get(&job).copied().unwrap_or(0);
            if used > total {
                problems.push(format!("{} uses {used} SP of {total}", jobs::name(job)));
            }
            Pool {
                job,
                job_name: jobs::name(job),
                total,
                spent: used,
            }
        })
        .collect();

    Evaluation { pools, problems }
}

/// Parse `id:level,id:level` (as used in share links and the CLI).
pub fn parse_build(s: &str) -> Result<BTreeMap<String, u32>, String> {
    s.split(',')
        .filter(|p| !p.trim().is_empty())
        .map(|pair| {
            let (id, lv) = pair
                .split_once([':', '.'])
                .ok_or_else(|| format!("expected id:level, got {pair:?}"))?;
            let lv = lv
                .trim()
                .parse()
                .map_err(|_| format!("bad level in {pair:?}"))?;
            Ok((id.trim().to_owned(), lv))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_pools() {
        let r = SpRules::default();
        // Level 30 warrior: beginner 6, 1st job 1 + 3 x 20 = 61.
        assert_eq!(r.pools(100, 30), [(0, 6), (100, 61)]);
        // After 2nd job at 30, 1st job stops earning.
        assert_eq!(r.pools(110, 70), [(0, 6), (100, 61), (110, 121)]);
        assert_eq!(r.pools(111, 70), [(0, 6), (100, 61), (110, 121), (111, 1)]);
        // Magicians advance at 8.
        assert_eq!(r.pools(200, 30), [(0, 6), (200, 67)]);
        assert_eq!(r.pools(110, 20), [(0, 6), (100, 31), (110, 0)]);
    }

    fn skill(id: &str, max: u32, req: &[(&str, u32)]) -> Skill {
        Skill {
            id: id.into(),
            job: jobs::of_skill(id.parse().unwrap()),
            name: Some(format!("S{id}")),
            max_level: max,
            req: req.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn evaluate_checks_rules() {
        let skills = [
            skill("1000000", 16, &[]),
            skill("1001003", 20, &[("1000000", 5)]),
        ];
        let r = SpRules::default();
        let ok = parse_build("1000000:16,1001003:20").unwrap();
        let e = evaluate(&r, 100, 30, &ok, &skills);
        assert!(e.problems.is_empty(), "{:?}", e.problems);
        assert_eq!(e.pools[1].spent, 36);

        let bad = parse_build("1000000:2,1001003:20,2001002:1").unwrap();
        let e = evaluate(&r, 100, 30, &bad, &skills);
        assert_eq!(e.problems.len(), 2, "{:?}", e.problems);
    }
}
