//! One record per `.img` in the whole client, hashed over its full
//! canonical JSON. This is the catch-all: any change anywhere in the data
//! shows up here, even for kinds without a dedicated extractor.
//!
//! Limitation: canvases are hashed by size and format, not pixels (pixel
//! data is not exposed without decoding every image). A sprite redrawn at
//! the same size shows up only in the `file` records of its `.wz`.

use anyhow::Result;
use rayon::prelude::*;

use super::Extractor;
use crate::record::{Record, hash_json};
use crate::wz::{self, JsonOptions, WzTree};

pub struct ImageHashes;

impl Extractor for ImageHashes {
    fn name(&self) -> &'static str {
        "images"
    }

    fn description(&self) -> &'static str {
        "content hash of every .img in the client (kind `img`)"
    }

    fn extract(&self, tree: &WzTree) -> Result<Vec<Record>> {
        let images = tree.images()?;
        tracing::info!(images = images.len(), "hashing images");
        let opts = JsonOptions {
            content_hashes: true,
            max_depth: None,
        };
        images
            .par_iter()
            .map(|(path, node)| {
                wz::parse(node)?;
                let hash = hash_json(&wz::node_to_json(node, opts));
                wz::unparse(node);
                Ok(Record::from_hash("img", path.clone(), hash))
            })
            .collect()
    }
}
