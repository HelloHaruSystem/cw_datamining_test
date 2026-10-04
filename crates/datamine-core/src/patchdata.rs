//! Nexon launcher patch manifests (`patchdata/<sha1>` files).
//!
//! Each manifest is zlib-compressed JSON listing every client file with its
//! size, mtime and object hashes. File names are base64 of UTF-16LE with a
//! BOM. `<product>.manifest.hash` names the manifest of the installed build.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use base64::Engine;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Manifest {
    pub build_time: f64,
    pub product: String,
    pub version: String,
    /// Decoded relative path (with `\` separators, as shipped) -> entry.
    pub files: BTreeMap<String, ManifestFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestFile {
    pub fsize: u64,
    pub mtime: i64,
    pub objects: Vec<String>,
}

impl ManifestFile {
    pub fn is_dir(&self) -> bool {
        self.objects.iter().any(|o| o == "__DIR__")
    }
}

#[derive(Deserialize)]
struct RawManifest {
    buildtime: f64,
    product: String,
    version: String,
    files: BTreeMap<String, ManifestFile>,
}

pub fn read_manifest(path: &Path) -> Result<Manifest> {
    let compressed = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    parse_manifest(&compressed).with_context(|| format!("parsing manifest {}", path.display()))
}

pub fn parse_manifest(compressed: &[u8]) -> Result<Manifest> {
    let mut json = Vec::new();
    flate2::read::ZlibDecoder::new(compressed).read_to_end(&mut json)?;
    let raw: RawManifest = serde_json::from_slice(&json)?;
    let files = raw
        .files
        .into_iter()
        .map(|(name, file)| Ok((decode_path(&name)?, file)))
        .collect::<Result<_>>()?;
    Ok(Manifest {
        build_time: raw.buildtime,
        product: raw.product,
        version: raw.version,
        files,
    })
}

/// The manifest of the installed build: `(manifest hash, manifest)`.
pub fn current_manifest(patchdata_dir: &Path) -> Result<Option<(String, Manifest)>> {
    for entry in std::fs::read_dir(patchdata_dir)? {
        let path = entry?.path();
        let is_pointer = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".manifest.hash"));
        if !is_pointer {
            continue;
        }
        let hash = std::fs::read_to_string(&path)?.trim().to_owned();
        let manifest = read_manifest(&patchdata_dir.join(&hash))?;
        return Ok(Some((hash, manifest)));
    }
    Ok(None)
}

fn decode_path(encoded: &str) -> Result<String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
    let bytes = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(&bytes);
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    Ok(String::from_utf16(&units)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn decodes_utf16_paths() {
        // "Canvas.dll" as shipped in real manifests.
        assert_eq!(
            decode_path("//5DAGEAbgB2AGEAcwAuAGQAbABsAA==").unwrap(),
            "Canvas.dll"
        );
    }

    #[test]
    fn parses_compressed_manifest() {
        let json = r#"{"buildtime": 1.5, "product": "59822", "version": "0.5",
            "files": {"//5EAGEAdABhAA==": {"fsize": 4096, "mtime": 1, "objects": ["__DIR__"]}}}"#;
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), Default::default());
        enc.write_all(json.as_bytes()).unwrap();
        let m = parse_manifest(&enc.finish().unwrap()).unwrap();
        assert_eq!(m.product, "59822");
        assert!(m.files["Data"].is_dir());
    }
}
