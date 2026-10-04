//! Importing a client into the store.
//!
//! The source client is **only ever read**: files are streamed into the
//! object store and all later work (parsing, extraction, browsing) happens
//! on the snapshot copy, never on the live install.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use rayon::prelude::*;
use serde_json::json;
use walkdir::WalkDir;

use crate::db::{NewVersion, Version};
use crate::objects::{Ingested, ObjectStore};
use crate::patchdata;
use crate::record::Record;
use crate::store::{Store, validate_label};

/// Extractor name under which file records are stored.
pub const SOURCE: &str = "import";

/// Top-level client entries that get copied. `Data/` is everything we
/// parse; the exe is kept so a snapshot records exactly which build it is.
pub const CLIENT_INCLUDE: &[&str] = &["Data", "MapleStory.exe"];

#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Client install directory (the one containing `Data/`).
    pub client_dir: PathBuf,
    /// Launcher `patchdata/` directory, if available.
    pub patchdata_dir: Option<PathBuf>,
    pub label: String,
    /// Free-form release channel, e.g. `prerelease`, `live`, `test`.
    pub channel: String,
    pub note: Option<String>,
}

#[derive(Debug)]
pub struct ImportReport {
    pub version: Version,
    pub files: usize,
    pub bytes: u64,
    /// Bytes not already present in the object store.
    pub new_bytes: u64,
    /// True if this import was made the baseline because none was set.
    pub became_baseline: bool,
}

pub fn import(store: &mut Store, opts: &ImportOptions) -> Result<ImportReport> {
    validate_label(&opts.label)?;
    if store.db().version_by_label(&opts.label)?.is_some() {
        bail!("a version labelled {:?} already exists", opts.label);
    }

    let client = canonical(&opts.client_dir)?;
    ensure!(
        client.join("Data/Base/Base.wz").is_file(),
        "{} does not look like a client: Data/Base/Base.wz not found",
        client.display()
    );
    let patchdata = opts.patchdata_dir.as_deref().map(canonical).transpose()?;
    let store_root = canonical(store.root())?;
    for src in std::iter::once(&client).chain(patchdata.as_ref()) {
        ensure!(
            !store_root.starts_with(src) && !src.starts_with(&store_root),
            "the store ({}) and source ({}) must not contain each other",
            store_root.display(),
            src.display()
        );
    }

    let mut files = collect_files(&client, CLIENT_INCLUDE, "")?;
    if let Some(dir) = &patchdata {
        files.extend(collect_files(dir, &[""], "patchdata/")?);
    }
    ensure!(!files.is_empty(), "nothing to import");
    tracing::info!(files = files.len(), "importing {}", client.display());

    let final_dir = store.root().join("snapshots").join(&opts.label);
    let staging = store
        .root()
        .join("snapshots")
        .join(format!(".{}.partial", opts.label));
    if staging.exists() {
        remove_tree(&staging)?;
    }

    let ingested = copy_into(store.objects(), &files, &staging).inspect_err(|_| {
        let _ = remove_tree(&staging);
    })?;
    fs::rename(&staging, &final_dir)
        .with_context(|| format!("finalizing snapshot {}", final_dir.display()))?;

    let result = record_version(store, opts, &client, patchdata.as_deref(), &ingested);
    if result.is_err() {
        let _ = remove_tree(&final_dir);
    }
    result
}

/// Ingest every file and lay out the snapshot tree under `staging`.
fn copy_into(
    objects: &ObjectStore,
    files: &[(String, PathBuf)],
    staging: &Path,
) -> Result<Vec<(String, Ingested)>> {
    files
        .par_iter()
        .map(|(rel, abs)| {
            let ingested = objects.ingest(abs)?;
            objects.link_into(&ingested.hash, &staging.join(rel))?;
            Ok((rel.clone(), ingested))
        })
        .collect()
}

fn record_version(
    store: &mut Store,
    opts: &ImportOptions,
    client: &Path,
    patchdata: Option<&Path>,
    ingested: &[(String, Ingested)],
) -> Result<ImportReport> {
    let manifest = match patchdata {
        Some(dir) => patchdata::current_manifest(dir)?,
        None => None,
    };
    let version = store.db().insert_version(&NewVersion {
        label: opts.label.clone(),
        channel: opts.channel.clone(),
        source_path: client.display().to_string(),
        manifest_hash: manifest.as_ref().map(|(hash, _)| hash.clone()),
        build_time: manifest
            .as_ref()
            .and_then(|(_, m)| build_time(m.build_time)),
        note: opts.note.clone(),
    })?;

    let records: Vec<Record> = ingested
        .iter()
        .map(|(rel, i)| Record {
            kind: "file".into(),
            key: rel.clone(),
            hash: i.hash.clone(),
            data: Some(json!({ "size": i.size })),
        })
        .collect();
    if let Err(e) = store.db_mut().replace_records(version.id, SOURCE, &records) {
        store.db().delete_version(version.id)?;
        return Err(e);
    }

    let became_baseline = store.baseline()?.is_none();
    if became_baseline {
        store.set_baseline(&version)?;
    }

    Ok(ImportReport {
        files: ingested.len(),
        bytes: ingested.iter().map(|(_, i)| i.size).sum(),
        new_bytes: ingested
            .iter()
            .filter(|(_, i)| i.is_new)
            .map(|(_, i)| i.size)
            .sum(),
        version,
        became_baseline,
    })
}

/// Regular files under `root/<include>` as `(prefix + relative path with
/// '/' separators, absolute path)`. Symlinks are not followed.
fn collect_files(root: &Path, include: &[&str], prefix: &str) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    for entry in include {
        let start = root.join(entry);
        if !start.exists() {
            tracing::warn!("{} not found, skipping", start.display());
            continue;
        }
        for item in WalkDir::new(&start).sort_by_file_name() {
            let item = item?;
            if !item.file_type().is_file() {
                continue;
            }
            let rel = item.path().strip_prefix(root)?;
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push((format!("{prefix}{rel}"), item.into_path()));
        }
    }
    Ok(out)
}

fn canonical(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("{} not found", path.display()))
}

fn build_time(unix: f64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis((unix * 1000.0) as i64).map(|t| t.to_rfc3339())
}

/// Snapshot files are read-only; directories are not, so plain removal works.
fn remove_tree(path: &Path) -> Result<()> {
    fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_client(dir: &Path) {
        fs::create_dir_all(dir.join("Data/Base")).unwrap();
        fs::create_dir_all(dir.join("Data/String")).unwrap();
        fs::write(dir.join("Data/Base/Base.wz"), b"base").unwrap();
        fs::write(dir.join("Data/String/String_000.wz"), b"strings").unwrap();
        fs::write(dir.join("MapleStory.exe"), b"exe").unwrap();
        fs::write(dir.join("Canvas.dll"), b"not copied").unwrap();
    }

    fn opts(client: &Path, label: &str) -> ImportOptions {
        ImportOptions {
            client_dir: client.into(),
            patchdata_dir: None,
            label: label.into(),
            channel: "test".into(),
            note: None,
        }
    }

    #[test]
    fn imports_copy_and_dedupes_second_version() {
        let tmp = tempfile::tempdir().unwrap();
        let client = tmp.path().join("client");
        fake_client(&client);
        let mut store = Store::open(tmp.path().join("store")).unwrap();

        let first = import(&mut store, &opts(&client, "v1")).unwrap();
        assert_eq!(first.files, 3);
        assert!(first.became_baseline);
        let snap = store.snapshot_dir(&first.version);
        assert_eq!(
            fs::read(snap.join("Data/String/String_000.wz")).unwrap(),
            b"strings"
        );
        assert!(!snap.join("Canvas.dll").exists());

        fs::write(client.join("Data/String/String_000.wz"), b"strings v2").unwrap();
        let second = import(&mut store, &opts(&client, "v2")).unwrap();
        assert!(!second.became_baseline);
        assert_eq!(second.new_bytes, "strings v2".len() as u64);

        let diff = store
            .db()
            .diff(first.version.id, second.version.id, &Default::default())
            .unwrap();
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].key, "Data/String/String_000.wz");
    }

    #[test]
    fn rejects_duplicate_label_and_overlapping_store() {
        let tmp = tempfile::tempdir().unwrap();
        let client = tmp.path().join("client");
        fake_client(&client);

        let mut inside = Store::open(client.join("store")).unwrap();
        assert!(import(&mut inside, &opts(&client, "v1")).is_err());

        let mut store = Store::open(tmp.path().join("store")).unwrap();
        import(&mut store, &opts(&client, "v1")).unwrap();
        assert!(import(&mut store, &opts(&client, "v1")).is_err());
    }
}
