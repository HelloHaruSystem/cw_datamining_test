//! Extractors turn a snapshot's WZ tree into [`Record`]s.
//!
//! To track a new kind of data, implement [`Extractor`] in a new module
//! and add it to [`registry`]. Each extractor owns its records: running it
//! again on a version atomically replaces what it produced before.

mod images;
pub mod skills;
mod strings;

use std::time::Instant;

use anyhow::{Result, bail};

use crate::db::Version;
use crate::record::Record;
use crate::store::Store;
use crate::wz::WzTree;

pub trait Extractor: Sync {
    /// Stable identifier, stored with every record it produces.
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn extract(&self, tree: &WzTree) -> Result<Vec<Record>>;
}

/// All extractors, in the order they run.
pub fn registry() -> Vec<Box<dyn Extractor>> {
    vec![
        Box::new(strings::Strings),
        Box::new(skills::Skills),
        Box::new(images::ImageHashes),
    ]
}

/// The WZ node a record was extracted from, for linking a record to the
/// tree browser. `None` for records without a node (e.g. files).
pub fn wz_path(kind: &str, key: &str) -> Option<String> {
    match kind {
        "img" => Some(key.to_owned()),
        skills::KIND => {
            let job = crate::jobs::of_skill(key.parse().ok()?);
            Some(format!("Skill/{job:03}.img/skill/{key}"))
        }
        k => k
            .strip_prefix("string/")
            .map(|img| format!("String/{img}.img/{key}")),
    }
}

#[derive(Debug)]
pub struct ExtractSummary {
    pub extractor: &'static str,
    pub records: usize,
    pub seconds: f64,
}

/// Run extractors on `version`'s snapshot and store the results.
/// `only` limits which extractors run (by name); empty means all.
pub fn run(store: &mut Store, version: &Version, only: &[String]) -> Result<Vec<ExtractSummary>> {
    let extractors: Vec<_> = registry()
        .into_iter()
        .filter(|e| only.is_empty() || only.iter().any(|o| o == e.name()))
        .collect();
    if extractors.is_empty() {
        let names: Vec<_> = registry().iter().map(|e| e.name()).collect();
        bail!(
            "no extractor matches {only:?}; available: {}",
            names.join(", ")
        );
    }

    let tree = WzTree::open(&store.snapshot_dir(version))?;
    let mut summaries = Vec::new();
    for extractor in extractors {
        let started = Instant::now();
        tracing::info!(
            extractor = extractor.name(),
            version = version.label,
            "extracting"
        );
        let records = extractor.extract(&tree)?;
        store
            .db_mut()
            .replace_records(version.id, extractor.name(), &records)?;
        summaries.push(ExtractSummary {
            extractor: extractor.name(),
            records: records.len(),
            seconds: started.elapsed().as_secs_f64(),
        });
    }
    Ok(summaries)
}
