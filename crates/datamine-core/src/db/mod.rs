//! Persistence layer.
//!
//! This is the **only** module that contains SQL. Everything else talks to
//! the database through the typed methods on [`Db`], so swapping SQLite for
//! another backend (e.g. Postgres) means reimplementing this module only.
//! Keep queries to portable SQL (`ON CONFLICT`, `IS DISTINCT FROM`,
//! `FULL OUTER JOIN`) so they translate with minimal changes.

mod migrations;

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;

use crate::record::{Hash, Record, hash_json};

pub type VersionId = i64;

#[derive(Debug, Clone, Serialize)]
pub struct Version {
    pub id: VersionId,
    pub label: String,
    pub channel: String,
    pub imported_at: String,
    pub source_path: String,
    pub manifest_hash: Option<String>,
    pub build_time: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NewVersion {
    pub label: String,
    pub channel: String,
    pub source_path: String,
    pub manifest_hash: Option<String>,
    pub build_time: Option<String>,
    pub note: Option<String>,
}

/// One record as stored for a single version.
#[derive(Debug, Clone, Serialize)]
pub struct StoredRecord {
    pub version_id: VersionId,
    pub kind: String,
    pub key: String,
    pub hash: Hash,
    pub data: Option<Value>,
}

/// A record that differs between two versions. `None` on a side means the
/// record does not exist in that version.
#[derive(Debug, Clone, Serialize)]
pub struct RecordPair {
    pub kind: String,
    pub key: String,
    pub old_hash: Option<Hash>,
    pub new_hash: Option<Hash>,
    pub old_data: Option<Value>,
    pub new_data: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Extraction {
    pub extractor: String,
    pub record_count: i64,
    pub finished_at: String,
}

/// Filters which records a query touches.
#[derive(Debug, Clone, Default)]
pub struct RecordFilter<'a> {
    /// Matches `kind` exactly or as a `/`-separated prefix, so `"string"`
    /// matches `"string/Eqp"`.
    pub kind: Option<&'a str>,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening database {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;",
        )?;
        let mut db = Self { conn };
        migrations::run(&mut db.conn)?;
        Ok(db)
    }

    // ---- versions -------------------------------------------------------

    pub fn insert_version(&self, v: &NewVersion) -> Result<Version> {
        let imported_at = now();
        self.conn
            .execute(
                "INSERT INTO versions
                     (label, channel, imported_at, source_path, manifest_hash, build_time, note)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    v.label,
                    v.channel,
                    imported_at,
                    v.source_path,
                    v.manifest_hash,
                    v.build_time,
                    v.note
                ],
            )
            .with_context(|| format!("inserting version {:?}", v.label))?;
        let id = self.conn.last_insert_rowid();
        self.version_by_id(id)?
            .context("version vanished right after insert")
    }

    /// All versions in release order: by build time from the patch manifest,
    /// falling back to import time, so an older build imported later still
    /// sorts first. Both are RFC 3339 UTC strings, which sort correctly.
    pub fn versions(&self) -> Result<Vec<Version>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {VERSION_COLS} FROM versions
             ORDER BY COALESCE(build_time, imported_at), id"
        ))?;
        let rows = stmt.query_map([], version_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn version_by_id(&self, id: VersionId) -> Result<Option<Version>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {VERSION_COLS} FROM versions WHERE id = ?1"),
                [id],
                version_from_row,
            )
            .optional()?)
    }

    pub fn version_by_label(&self, label: &str) -> Result<Option<Version>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {VERSION_COLS} FROM versions WHERE label = ?1"),
                [label],
                version_from_row,
            )
            .optional()?)
    }

    pub fn delete_version(&self, id: VersionId) -> Result<()> {
        self.conn
            .execute("DELETE FROM versions WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Hashes of every stored file (`file` records) across all versions.
    pub fn referenced_file_hashes(&self) -> Result<std::collections::HashSet<Hash>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT hash FROM records WHERE kind = 'file'")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Delete blobs no record points to. Returns how many were deleted.
    pub fn delete_orphan_blobs(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM blobs WHERE hash NOT IN
                 (SELECT blob_hash FROM records WHERE blob_hash IS NOT NULL)",
            [],
        )?)
    }

    pub fn delete_meta(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM meta WHERE key = ?1", [key])?;
        Ok(())
    }

    // ---- meta -----------------------------------------------------------

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ---- records --------------------------------------------------------

    /// Atomically replace everything `source` produced for `version` and
    /// mark the extraction as finished.
    pub fn replace_records(
        &mut self,
        version: VersionId,
        source: &str,
        records: &[Record],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM records WHERE version_id = ?1 AND source = ?2",
            params![version, source],
        )?;
        {
            let mut put_blob = tx.prepare(
                "INSERT INTO blobs (hash, data) VALUES (?1, ?2)
                 ON CONFLICT (hash) DO NOTHING",
            )?;
            let mut put_record = tx.prepare(
                "INSERT INTO records (version_id, source, kind, key, hash, blob_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for r in records {
                let blob_hash = match &r.data {
                    Some(data) => {
                        let h = hash_json(data);
                        put_blob.execute(params![h, data.to_string()])?;
                        Some(h)
                    }
                    None => None,
                };
                put_record
                    .execute(params![version, source, r.kind, r.key, r.hash, blob_hash])
                    .with_context(|| format!("inserting record {}:{}", r.kind, r.key))?;
            }
        }
        tx.execute(
            "INSERT INTO extractions (version_id, extractor, record_count, finished_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (version_id, extractor) DO UPDATE
                 SET record_count = excluded.record_count,
                     finished_at  = excluded.finished_at",
            params![version, source, records.len() as i64, now()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn extractions(&self, version: VersionId) -> Result<Vec<Extraction>> {
        let mut stmt = self.conn.prepare(
            "SELECT extractor, record_count, finished_at FROM extractions
             WHERE version_id = ?1 ORDER BY extractor",
        )?;
        let rows = stmt.query_map([version], |r| {
            Ok(Extraction {
                extractor: r.get(0)?,
                record_count: r.get(1)?,
                finished_at: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Record counts per kind for one version.
    pub fn kind_counts(&self, version: VersionId) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, COUNT(*) FROM records WHERE version_id = ?1
             GROUP BY kind ORDER BY kind",
        )?;
        let rows = stmt.query_map([version], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Ids of versions that have at least one record of `kind`.
    pub fn versions_with_kind(&self, kind: &str) -> Result<Vec<VersionId>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT version_id FROM records WHERE kind = ?1 ORDER BY version_id",
        )?;
        let rows = stmt.query_map([kind], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Records that are added, removed or changed between two versions.
    pub fn diff(
        &self,
        from: VersionId,
        to: VersionId,
        filter: &RecordFilter,
    ) -> Result<Vec<RecordPair>> {
        let (kind_eq, kind_like) = kind_params(filter);
        // Two halves instead of a FULL OUTER JOIN over CTEs, which SQLite
        // runs as a nested loop. Each half probes the other version through
        // the (version_id, kind, key) primary key.
        let a_filter = KIND_FILTER.replace("kind", "a.kind");
        let b_filter = KIND_FILTER.replace("kind", "b.kind");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT a.kind, a.key, a.hash, b.hash, ba.data, bb.data
             FROM records a
             LEFT JOIN records b
                    ON b.version_id = ?2 AND b.kind = a.kind AND b.key = a.key
             LEFT JOIN blobs ba ON ba.hash = a.blob_hash
             LEFT JOIN blobs bb ON bb.hash = b.blob_hash
             WHERE a.version_id = ?1 AND {a_filter}
               AND a.hash IS DISTINCT FROM b.hash
             UNION ALL
             SELECT b.kind, b.key, NULL, b.hash, NULL, bb.data
             FROM records b
             LEFT JOIN blobs bb ON bb.hash = b.blob_hash
             WHERE b.version_id = ?2 AND {b_filter}
               AND NOT EXISTS (SELECT 1 FROM records a
                               WHERE a.version_id = ?1 AND a.kind = b.kind AND a.key = b.key)
             ORDER BY 1, 2"
        ))?;
        let rows = stmt.query_map(params![from, to, kind_eq, kind_like], |r| {
            Ok(RecordPair {
                kind: r.get(0)?,
                key: r.get(1)?,
                old_hash: r.get(2)?,
                new_hash: r.get(3)?,
                old_data: json_col(r, 4)?,
                new_data: json_col(r, 5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every stored state of records whose key equals `key` or ends in
    /// `/<key>`, across all versions, ordered by kind, key, version.
    pub fn history(&self, key: &str, filter: &RecordFilter) -> Result<Vec<StoredRecord>> {
        let (kind_eq, kind_like) = kind_params(filter);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT r.version_id, r.kind, r.key, r.hash, b.data
             FROM records r LEFT JOIN blobs b ON b.hash = r.blob_hash
             WHERE (r.key = ?3 OR r.key LIKE ?4 ESCAPE '\\') AND {KIND_FILTER_R}
             ORDER BY r.kind, r.key, r.version_id"
        ))?;
        let suffix = format!("%/{}", escape_like(key));
        let rows = stmt.query_map(params![kind_eq, kind_like, key, suffix], stored_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// All records of one version matching `filter`, ordered by kind, key.
    pub fn records(&self, version: VersionId, filter: &RecordFilter) -> Result<Vec<StoredRecord>> {
        let (kind_eq, kind_like) = kind_params(filter);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT r.version_id, r.kind, r.key, r.hash, b.data
             FROM records r LEFT JOIN blobs b ON b.hash = r.blob_hash
             WHERE r.version_id = ?3 AND {KIND_FILTER_R}
             ORDER BY r.kind, r.key"
        ))?;
        let rows = stmt.query_map(params![kind_eq, kind_like, version], stored_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One record by exact kind and key.
    pub fn record(
        &self,
        version: VersionId,
        kind: &str,
        key: &str,
    ) -> Result<Option<StoredRecord>> {
        Ok(self
            .conn
            .query_row(
                "SELECT r.version_id, r.kind, r.key, r.hash, b.data
                 FROM records r LEFT JOIN blobs b ON b.hash = r.blob_hash
                 WHERE r.version_id = ?1 AND r.kind = ?2 AND r.key = ?3",
                params![version, kind, key],
                stored_from_row,
            )
            .optional()?)
    }

    /// Records of one version whose key or payload contains `text`
    /// (case-insensitive for ASCII).
    pub fn search(
        &self,
        version: VersionId,
        text: &str,
        filter: &RecordFilter,
        limit: usize,
    ) -> Result<Vec<StoredRecord>> {
        let (kind_eq, kind_like) = kind_params(filter);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT r.version_id, r.kind, r.key, r.hash, b.data
             FROM records r LEFT JOIN blobs b ON b.hash = r.blob_hash
             WHERE r.version_id = ?3 AND {KIND_FILTER_R}
               AND (r.key LIKE ?4 ESCAPE '\\' OR b.data LIKE ?4 ESCAPE '\\')
             ORDER BY r.kind, r.key
             LIMIT ?5"
        ))?;
        let pattern = format!("%{}%", escape_like(text));
        let rows = stmt.query_map(
            params![kind_eq, kind_like, version, pattern, limit as i64],
            stored_from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

// ---- helpers ------------------------------------------------------------

const VERSION_COLS: &str =
    "id, label, channel, imported_at, source_path, manifest_hash, build_time, note";

/// Kind filter bound to the two trailing params of each query (`?3`/`?4`
/// for diff after the version ids, `?1`/`?2` for the others). A NULL
/// kind means "no filter".
const KIND_FILTER: &str = "(?3 IS NULL OR kind = ?3 OR kind LIKE ?4 ESCAPE '\\')";
const KIND_FILTER_R: &str = "(?1 IS NULL OR r.kind = ?1 OR r.kind LIKE ?2 ESCAPE '\\')";

fn kind_params(filter: &RecordFilter) -> (Option<String>, Option<String>) {
    match filter.kind {
        Some(k) => (Some(k.to_owned()), Some(format!("{}/%", escape_like(k)))),
        None => (None, None),
    }
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn version_from_row(r: &rusqlite::Row) -> rusqlite::Result<Version> {
    Ok(Version {
        id: r.get(0)?,
        label: r.get(1)?,
        channel: r.get(2)?,
        imported_at: r.get(3)?,
        source_path: r.get(4)?,
        manifest_hash: r.get(5)?,
        build_time: r.get(6)?,
        note: r.get(7)?,
    })
}

fn stored_from_row(r: &rusqlite::Row) -> rusqlite::Result<StoredRecord> {
    Ok(StoredRecord {
        version_id: r.get(0)?,
        kind: r.get(1)?,
        key: r.get(2)?,
        hash: r.get(3)?,
        data: json_col(r, 4)?,
    })
}

fn json_col(r: &rusqlite::Row, idx: usize) -> rusqlite::Result<Option<Value>> {
    let text: Option<String> = r.get(idx)?;
    text.map(|t| {
        serde_json::from_str(&t).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, e.into())
        })
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn version(db: &Db, label: &str) -> VersionId {
        db.insert_version(&NewVersion {
            label: label.into(),
            channel: "test".into(),
            source_path: "/src".into(),
            ..Default::default()
        })
        .unwrap()
        .id
    }

    fn rec(kind: &str, key: &str, name: &str) -> Record {
        Record::from_data(kind, key, json!({ "name": name }))
    }

    #[test]
    fn versions_sort_by_build_time_not_import_order() {
        let db = Db::open_in_memory().unwrap();
        for (label, build) in [
            ("newer", "2026-08-11T23:07:05+00:00"),
            ("older", "2026-04-20T21:30:31+00:00"),
        ] {
            db.insert_version(&NewVersion {
                label: label.into(),
                channel: "test".into(),
                source_path: "/src".into(),
                build_time: Some(build.into()),
                ..Default::default()
            })
            .unwrap();
        }
        let labels: Vec<_> = db
            .versions()
            .unwrap()
            .into_iter()
            .map(|v| v.label)
            .collect();
        assert_eq!(labels, ["older", "newer"]);
    }

    #[test]
    fn diff_reports_added_removed_and_modified() {
        let mut db = Db::open_in_memory().unwrap();
        let a = version(&db, "a");
        let b = version(&db, "b");
        db.replace_records(
            a,
            "t",
            &[
                rec("s/Eqp", "1", "Old"),
                rec("s/Eqp", "2", "Gone"),
                rec("s/Eqp", "3", "Same"),
            ],
        )
        .unwrap();
        db.replace_records(
            b,
            "t",
            &[
                rec("s/Eqp", "1", "New"),
                rec("s/Eqp", "3", "Same"),
                rec("s/Eqp", "4", "Added"),
            ],
        )
        .unwrap();

        let diff = db.diff(a, b, &RecordFilter::default()).unwrap();
        let summary: Vec<_> = diff
            .iter()
            .map(|p| (p.key.as_str(), p.old_hash.is_some(), p.new_hash.is_some()))
            .collect();
        assert_eq!(
            summary,
            [("1", true, true), ("2", true, false), ("4", false, true)]
        );
        assert_eq!(diff[0].new_data, Some(json!({ "name": "New" })));
    }

    #[test]
    fn kind_filter_matches_prefix_only_on_segment_boundary() {
        let mut db = Db::open_in_memory().unwrap();
        let a = version(&db, "a");
        let b = version(&db, "b");
        db.replace_records(
            b,
            "t",
            &[rec("string/Eqp", "1", "x"), rec("stringy", "1", "y")],
        )
        .unwrap();

        let diff = db
            .diff(
                a,
                b,
                &RecordFilter {
                    kind: Some("string"),
                },
            )
            .unwrap();
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, "string/Eqp");
    }

    #[test]
    fn replace_records_is_idempotent_per_source() {
        let mut db = Db::open_in_memory().unwrap();
        let a = version(&db, "a");
        db.replace_records(a, "t", &[rec("k", "1", "x")]).unwrap();
        db.replace_records(a, "t", &[rec("k", "1", "x")]).unwrap();
        assert_eq!(db.kind_counts(a).unwrap(), [("k".to_string(), 1)]);
    }

    #[test]
    fn history_matches_key_suffix() {
        let mut db = Db::open_in_memory().unwrap();
        let a = version(&db, "a");
        db.replace_records(
            a,
            "t",
            &[
                rec("s/Eqp", "Eqp/Weapon/1302000", "Sword"),
                rec("s/Eqp", "Eqp/Weapon/11302000", "Other"),
            ],
        )
        .unwrap();
        let hist = db.history("1302000", &RecordFilter::default()).unwrap();
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].key, "Eqp/Weapon/1302000");
    }
}
