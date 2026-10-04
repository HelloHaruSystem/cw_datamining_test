//! Explorer job tree. The client stores skills per job id (`Skill/110.img`)
//! but not job names or how jobs relate, so that part lives here.
//!
//! Job ids follow a fixed scheme: `B00` is the 1st job of branch `B`,
//! `B10`/`B20`/`B30` are 2nd jobs, and the last digit counts further
//! advancements (`111` = 3rd job after `110`).

use serde::Serialize;

pub type JobId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Branch {
    Beginner,
    Warrior,
    Magician,
    Bowman,
    Thief,
    Gm,
}

impl Branch {
    pub const ALL: [Branch; 6] = [
        Branch::Beginner,
        Branch::Warrior,
        Branch::Magician,
        Branch::Bowman,
        Branch::Thief,
        Branch::Gm,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Branch::Beginner => "beginner",
            Branch::Warrior => "warrior",
            Branch::Magician => "magician",
            Branch::Bowman => "bowman",
            Branch::Thief => "thief",
            Branch::Gm => "gm",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Branch::Beginner => "Beginner",
            Branch::Warrior => "Warrior",
            Branch::Magician => "Magician",
            Branch::Bowman => "Bowman",
            Branch::Thief => "Thief",
            Branch::Gm => "GM",
        }
    }

    pub fn from_slug(s: &str) -> Option<Branch> {
        Branch::ALL
            .into_iter()
            .find(|b| b.slug().eq_ignore_ascii_case(s))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Job {
    pub id: JobId,
    pub name: &'static str,
    pub branch: Branch,
    /// 0 = beginner, 1..=4 = job advancement.
    pub advancement: u8,
}

const fn job(id: JobId, name: &'static str, branch: Branch, advancement: u8) -> Job {
    Job {
        id,
        name,
        branch,
        advancement,
    }
}

use Branch::*;

pub const JOBS: &[Job] = &[
    job(0, "Beginner", Beginner, 0),
    job(100, "Warrior", Warrior, 1),
    job(110, "Fighter", Warrior, 2),
    job(111, "Crusader", Warrior, 3),
    job(112, "Hero", Warrior, 4),
    job(120, "Page", Warrior, 2),
    job(121, "White Knight", Warrior, 3),
    job(122, "Paladin", Warrior, 4),
    job(130, "Spearman", Warrior, 2),
    job(131, "Dragon Knight", Warrior, 3),
    job(132, "Dark Knight", Warrior, 4),
    job(200, "Magician", Magician, 1),
    job(210, "Wizard (Fire, Poison)", Magician, 2),
    job(211, "Mage (Fire, Poison)", Magician, 3),
    job(212, "Arch Mage (Fire, Poison)", Magician, 4),
    job(220, "Wizard (Ice, Lightning)", Magician, 2),
    job(221, "Mage (Ice, Lightning)", Magician, 3),
    job(222, "Arch Mage (Ice, Lightning)", Magician, 4),
    job(230, "Cleric", Magician, 2),
    job(231, "Priest", Magician, 3),
    job(232, "Bishop", Magician, 4),
    job(300, "Bowman", Bowman, 1),
    job(310, "Hunter", Bowman, 2),
    job(311, "Ranger", Bowman, 3),
    job(312, "Bowmaster", Bowman, 4),
    job(320, "Crossbowman", Bowman, 2),
    job(321, "Sniper", Bowman, 3),
    job(322, "Marksman", Bowman, 4),
    job(400, "Thief", Thief, 1),
    job(410, "Assassin", Thief, 2),
    job(411, "Hermit", Thief, 3),
    job(412, "Night Lord", Thief, 4),
    job(420, "Bandit", Thief, 2),
    job(421, "Chief Bandit", Thief, 3),
    job(422, "Shadower", Thief, 4),
    job(900, "GM", Gm, 1),
    job(910, "Super GM", Gm, 2),
];

pub fn get(id: JobId) -> Option<&'static Job> {
    JOBS.iter().find(|j| j.id == id)
}

/// Job that owns a skill: skill ids are `job * 10000 + n`.
pub fn of_skill(skill_id: u32) -> JobId {
    skill_id / 10000
}

/// The job before `id` in its line (`111` -> `110` -> `100` -> `0`).
pub fn parent(id: JobId) -> Option<JobId> {
    match id {
        0 => None,
        _ if id % 100 == 0 => Some(0),
        _ if id % 10 == 0 => Some(id / 100 * 100),
        _ => Some(id - 1),
    }
}

/// Every job from beginner up to `id`, in advancement order.
pub fn path(id: JobId) -> Vec<JobId> {
    let mut out = vec![id];
    let mut cur = id;
    while let Some(p) = parent(cur) {
        out.push(p);
        cur = p;
    }
    out.reverse();
    out
}

/// Jobs that have nothing after them among `available`, i.e. the jobs a
/// skill build can target.
pub fn leaves(available: &[JobId]) -> Vec<JobId> {
    available
        .iter()
        .copied()
        .filter(|&j| !available.iter().any(|&o| o != j && parent(o) == Some(j)))
        .collect()
}

/// Job advancements: `(level, slug, label)`. Level 0 is beginner.
pub const ADVANCEMENTS: [(u8, &str, &str); 5] = [
    (0, "beginner", "Beginner"),
    (1, "1st", "1st job"),
    (2, "2nd", "2nd job"),
    (3, "3rd", "3rd job"),
    (4, "4th", "4th job"),
];

/// Advancement of a job (0 = beginner, 1..=4).
pub fn advancement(id: JobId) -> Option<u8> {
    get(id).map(|j| j.advancement)
}

/// `"2nd job"` for advancement 2.
pub fn advancement_label(advancement: u8) -> &'static str {
    ADVANCEMENTS
        .iter()
        .find(|(a, _, _)| *a == advancement)
        .map_or("Other", |(_, _, label)| label)
}

/// Parse `2nd`, `2`, `second` or `beginner`.
pub fn parse_advancement(s: &str) -> Option<u8> {
    let s = s.trim().to_ascii_lowercase();
    let words = ["beginner", "first", "second", "third", "fourth"];
    ADVANCEMENTS
        .iter()
        .find(|(a, slug, _)| s == *slug || s == a.to_string() || s == words[*a as usize])
        .map(|(a, _, _)| *a)
}

/// Job by name (`crusader`, case-insensitive) or id (`111`).
pub fn find(selector: &str) -> Option<JobId> {
    selector.parse().ok().or_else(|| {
        JOBS.iter()
            .find(|j| j.name.eq_ignore_ascii_case(selector))
            .map(|j| j.id)
    })
}

pub fn name(id: JobId) -> String {
    get(id).map_or_else(|| format!("Job {id}"), |j| j.name.to_owned())
}

pub fn branch(id: JobId) -> Option<Branch> {
    get(id).map(|j| j.branch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(path(111), [0, 100, 110, 111]);
        assert_eq!(path(230), [0, 200, 230]);
        assert_eq!(path(0), [0]);
        assert_eq!(of_skill(1101006), 110);
        assert_eq!(of_skill(1000), 0);
    }

    #[test]
    fn advancements() {
        assert_eq!(advancement(111), Some(3));
        assert_eq!(parse_advancement("2nd"), Some(2));
        assert_eq!(parse_advancement("Third"), Some(3));
        assert_eq!(parse_advancement("0"), Some(0));
        assert_eq!(parse_advancement("5th"), None);
        assert_eq!(advancement_label(1), "1st job");
    }

    #[test]
    fn leaves_are_last_advancements() {
        assert_eq!(leaves(&[0, 100, 110, 111, 120]), [111, 120]);
    }
}
