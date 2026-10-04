//! Content-addressed file storage.
//!
//! Every imported file is stored once under `objects/<aa>/<hash>`, where
//! `<hash>` is the blake3 of its contents, and made read-only. Snapshots
//! are trees of hard links into this store, so a `.wz` that didn't change
//! between patches costs no extra disk space, and no tool can accidentally
//! modify a snapshot in place.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tempfile::NamedTempFile;

use crate::record::Hash;

pub struct ObjectStore {
    dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Ingested {
    pub hash: Hash,
    pub size: u64,
    /// False if identical content was already stored.
    pub is_new: bool,
}

impl ObjectStore {
    pub fn new(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(dir.join("tmp"))?;
        Ok(Self { dir })
    }

    pub fn path(&self, hash: &str) -> PathBuf {
        self.dir.join(&hash[..2]).join(hash)
    }

    /// Delete every object whose hash is not in `keep`.
    /// Returns `(objects removed, bytes freed)`.
    pub fn retain(&self, keep: &HashSet<Hash>) -> Result<(usize, u64)> {
        let (mut count, mut bytes) = (0, 0);
        for shard in fs::read_dir(&self.dir)? {
            let shard = shard?;
            if shard.file_name() == "tmp" || !shard.file_type()?.is_dir() {
                continue;
            }
            for obj in fs::read_dir(shard.path())? {
                let obj = obj?;
                let name = obj.file_name();
                if keep.contains(name.to_string_lossy().as_ref()) {
                    continue;
                }
                let path = obj.path();
                bytes += obj.metadata()?.len();
                set_writable(&path)?;
                fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
                count += 1;
            }
        }
        Ok((count, bytes))
    }

    /// Copy `src` into the store. `src` is only ever opened for reading.
    pub fn ingest(&self, src: &Path) -> Result<Ingested> {
        let mut reader = File::open(src).with_context(|| format!("opening {}", src.display()))?;
        let tmp = NamedTempFile::new_in(self.dir.join("tmp"))?;
        let mut writer = HashingWriter {
            inner: io::BufWriter::new(tmp.as_file()),
            hasher: blake3::Hasher::new(),
        };
        let size = io::copy(&mut reader, &mut writer)
            .with_context(|| format!("copying {}", src.display()))?;
        writer.inner.flush()?;
        let hash = writer.hasher.finalize().to_hex().to_string();
        drop(writer);

        let dest = self.path(&hash);
        if dest.exists() {
            return Ok(Ingested {
                hash,
                size,
                is_new: false,
            });
        }
        fs::create_dir_all(dest.parent().expect("object path has a parent"))?;
        tmp.as_file().sync_all()?;
        tmp.persist(&dest)
            .with_context(|| format!("storing object {}", dest.display()))?;
        set_readonly(&dest)?;
        Ok(Ingested {
            hash,
            size,
            is_new: true,
        })
    }

    /// Materialize object `hash` at `dest`, as a hard link when possible.
    pub fn link_into(&self, hash: &str, dest: &Path) -> Result<()> {
        let src = self.path(hash);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        if fs::hard_link(&src, dest).is_err() {
            // Different filesystem or no hard link support: fall back to a copy.
            fs::copy(&src, dest)
                .with_context(|| format!("copying object to {}", dest.display()))?;
            set_readonly(dest)?;
        }
        Ok(())
    }
}

struct HashingWriter<W> {
    inner: W,
    hasher: blake3::Hasher,
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[allow(clippy::permissions_set_readonly_false)] // only ever applied to our own objects
fn set_writable(path: &Path) -> Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_readonly(false);
    fs::set_permissions(path, perms)?;
    Ok(())
}

fn set_readonly(path: &Path) -> Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_readonly(true);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_dedupes_and_links() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects")).unwrap();
        let src = tmp.path().join("a.wz");
        fs::write(&src, b"hello").unwrap();

        let first = store.ingest(&src).unwrap();
        let second = store.ingest(&src).unwrap();
        assert!(first.is_new);
        assert!(!second.is_new);
        assert_eq!(first.hash, blake3::hash(b"hello").to_hex().to_string());

        let dest = tmp.path().join("snap/Data/a.wz");
        store.link_into(&first.hash, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"hello");
        assert!(fs::metadata(&dest).unwrap().permissions().readonly());
        // The source is untouched.
        assert!(!fs::metadata(&src).unwrap().permissions().readonly());

        fs::remove_file(&dest).unwrap();
        assert_eq!(store.retain(&HashSet::new()).unwrap(), (1, 5));
        assert!(!store.path(&first.hash).exists());
    }
}
