//! The datamine store: a self-contained directory holding the database and
//! every imported client snapshot.
//!
//! ```text
//! store/
//!   datamine.db              SQLite: versions, records, history
//!   objects/aa/<blake3>      deduplicated, read-only file contents
//!   snapshots/<label>/       per-version tree of hard links into objects/
//! ```
//!
//! Nothing in the database refers to absolute paths inside the store, so
//! the whole directory can be moved or synced elsewhere (`rsync -aH`, to
//! keep hard links) and keep working.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::db::{Db, Version};
use crate::objects::ObjectStore;

const DB_FILE: &str = "datamine.db";
const BASELINE_KEY: &str = "baseline";

pub struct Store {
    root: PathBuf,
    db: Db,
    objects: ObjectStore,
}

impl Store {
    /// Open the store at `root`, creating it if needed.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(root.join("snapshots"))
            .with_context(|| format!("creating store at {}", root.display()))?;
        let objects = ObjectStore::new(root.join("objects"))?;
        let db = Db::open(&root.join(DB_FILE))?;
        Ok(Self { root, db, objects })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn db_mut(&mut self) -> &mut Db {
        &mut self.db
    }

    pub fn objects(&self) -> &ObjectStore {
        &self.objects
    }

    /// Directory holding the copied client files of `version`.
    pub fn snapshot_dir(&self, version: &Version) -> PathBuf {
        self.root.join("snapshots").join(&version.label)
    }

    pub fn baseline(&self) -> Result<Option<Version>> {
        match self.db.meta(BASELINE_KEY)? {
            Some(label) => self.db.version_by_label(&label),
            None => Ok(None),
        }
    }

    pub fn set_baseline(&self, version: &Version) -> Result<()> {
        self.db.set_meta(BASELINE_KEY, &version.label)
    }

    /// Delete a version: its records, its snapshot directory, and the
    /// baseline marker if it was the baseline. Run [`Store::gc`] afterwards
    /// to free file contents no other version uses.
    pub fn remove_version(&mut self, version: &Version) -> Result<()> {
        if self.baseline()?.is_some_and(|b| b.id == version.id) {
            self.db.delete_meta(BASELINE_KEY)?;
        }
        self.db.delete_version(version.id)?;
        let dir = self.snapshot_dir(version);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
        Ok(())
    }

    /// Delete stored files and blobs that no version references anymore.
    pub fn gc(&self) -> Result<GcReport> {
        let keep = self.db.referenced_file_hashes()?;
        let (objects, bytes) = self.objects.retain(&keep)?;
        let blobs = self.db.delete_orphan_blobs()?;
        Ok(GcReport {
            objects,
            bytes,
            blobs,
        })
    }

    /// The version released right before `version`, if any.
    pub fn previous(&self, version: &Version) -> Result<Option<Version>> {
        Ok(self
            .db
            .versions()?
            .into_iter()
            .take_while(|v| v.id != version.id)
            .last())
    }

    /// Default "old side" for a diff ending at `to`: the baseline if it was
    /// released before `to`, otherwise the version right before `to`.
    pub fn diff_base(&self, to: &Version) -> Result<Option<Version>> {
        let versions = self.db.versions()?;
        let pos = |id| versions.iter().position(|v| v.id == id);
        if let Some(b) = self.baseline()?
            && pos(b.id) < pos(to.id)
        {
            return Ok(Some(b));
        }
        self.previous(to)
    }

    /// Resolve a version selector:
    /// `latest`, `previous` (the one before latest), `baseline`, `#<id>`,
    /// or a label.
    pub fn resolve(&self, selector: &str) -> Result<Version> {
        let found = match selector {
            "latest" => self.db.versions()?.pop(),
            "previous" => {
                let mut all = self.db.versions()?;
                all.pop();
                all.pop()
            }
            "baseline" => self.baseline()?,
            s if s.starts_with('#') => {
                let id = s[1..]
                    .parse()
                    .with_context(|| format!("invalid version id {s:?}"))?;
                self.db.version_by_id(id)?
            }
            label => self.db.version_by_label(label)?,
        };
        match found {
            Some(v) => Ok(v),
            None => bail!("no version matches {selector:?} (see `datamine versions`)"),
        }
    }
}

#[derive(Debug)]
pub struct GcReport {
    pub objects: usize,
    pub bytes: u64,
    pub blobs: usize,
}

/// Labels become directory names, so keep them boring.
pub fn validate_label(label: &str) -> Result<()> {
    let ok = !label.is_empty()
        && !label.starts_with(['.', '#'])
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !ok || matches!(label, "latest" | "previous" | "baseline") {
        bail!(
            "invalid label {label:?}: use letters, digits, '.', '_' or '-', \
             and not a reserved selector (latest, previous, baseline)"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert!(validate_label("v260-prerelease").is_ok());
        assert!(validate_label("2026.10.06").is_ok());
        for bad in ["", "../x", "a b", "latest", ".hidden", "#3"] {
            assert!(validate_label(bad).is_err(), "{bad:?} should be rejected");
        }
    }
}
