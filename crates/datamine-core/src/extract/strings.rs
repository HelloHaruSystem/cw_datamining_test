//! String.wz entries: item, skill, map, mob, NPC, ... names and texts.
//!
//! Every `String/<X>.img` becomes kind `string/<X>`. An entry is any node
//! with a numeric name that holds at least one value, e.g.
//! `Eqp.img/Eqp/Weapon/1302000 { name, desc }`. Its key is its path inside
//! the image (`Eqp/Weapon/1302000`), and its data is its subtree minus any
//! nested entries (those become records of their own).

use anyhow::Result;
use rayon::prelude::*;
use serde_json::{Map, Value};
use wz_reader::WzNodeArc;

use super::Extractor;
use crate::record::Record;
use crate::wz::{self, JsonOptions, WzTree};

pub struct Strings;

impl Extractor for Strings {
    fn name(&self) -> &'static str {
        "strings"
    }

    fn description(&self) -> &'static str {
        "String.wz names and descriptions (kinds `string/<img>`)"
    }

    fn extract(&self, tree: &WzTree) -> Result<Vec<Record>> {
        let images: Vec<_> = tree
            .images()?
            .into_iter()
            .filter(|(path, _)| path.starts_with("String/"))
            .collect();
        let per_image: Vec<Vec<Record>> = images
            .par_iter()
            .map(|(path, node)| {
                wz::parse(node)?;
                let kind = format!(
                    "string/{}",
                    path.trim_start_matches("String/").trim_end_matches(".img")
                );
                let mut out = Vec::new();
                collect_entries(node, &kind, "", &mut out);
                wz::unparse(node);
                Ok(out)
            })
            .collect::<Result<_>>()?;
        Ok(per_image.into_iter().flatten().collect())
    }
}

fn collect_entries(node: &WzNodeArc, kind: &str, path: &str, out: &mut Vec<Record>) {
    for (name, child) in wz::children(node) {
        let child_path = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        if is_entry(&name, &child) {
            out.push(Record::from_data(
                kind,
                child_path.clone(),
                entry_data(&child),
            ));
        }
        collect_entries(&child, kind, &child_path, out);
    }
}

fn is_entry(name: &str, node: &WzNodeArc) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_digit())
        && node
            .read()
            .expect("poisoned lock")
            .children
            .values()
            .any(wz::is_value)
}

/// The entry's subtree as JSON, leaving out nested entries.
fn entry_data(node: &WzNodeArc) -> Value {
    let read = node.read().expect("poisoned lock");
    let mut obj = Map::new();
    for (name, child) in &read.children {
        if is_entry(name, child) {
            continue;
        }
        obj.insert(
            name.to_string(),
            wz::node_to_json(child, JsonOptions::default()),
        );
    }
    Value::Object(obj)
}
