//! Thin layer over `wz_reader`: open a snapshot's WZ tree, navigate it, and
//! turn nodes into canonical JSON (for display and for hashing).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Number, Value, json};
use wz_reader::property::{WzSubProperty, WzValue};
use wz_reader::util::resolve_base;
use wz_reader::{WzNodeCast, WzObjectType};

pub use wz_reader::WzNodeArc;

use crate::record::hash_bytes;

/// The merged WZ tree of one snapshot (`Data/Base/Base.wz` and everything
/// it references, including split `_NNN.wz` files).
pub struct WzTree {
    root: WzNodeArc,
}

impl WzTree {
    pub fn open(snapshot_dir: &Path) -> Result<Self> {
        let base = snapshot_dir.join("Data/Base/Base.wz");
        if !base.is_file() {
            bail!("{} not found", base.display());
        }
        // wz_reader panics on some malformed inputs; surface that as an error.
        let root = catch_unwind(AssertUnwindSafe(|| resolve_base(&base, None)))
            .map_err(|_| anyhow!("wz_reader panicked while opening {}", base.display()))?
            .with_context(|| format!("opening {}", base.display()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &WzNodeArc {
        &self.root
    }

    /// Node at a `/`-separated path such as `String/Eqp.img/Eqp/Weapon`,
    /// parsing every node on the way, including the target. The empty
    /// path is the root.
    pub fn get(&self, path: &str) -> Result<WzNodeArc> {
        let mut node = self.root.clone();
        for name in path.split('/').filter(|s| !s.is_empty()) {
            parse(&node)?;
            let child = node.read().expect("poisoned lock").at(name);
            node = child.with_context(|| format!("{path:?}: no node named {name:?}"))?;
        }
        parse(&node)?;
        Ok(node)
    }

    /// Every `.img` in the tree as `(path, node)`, without parsing image
    /// contents. Paths are relative to the root, e.g. `String/Eqp.img`.
    pub fn images(&self) -> Result<Vec<(String, WzNodeArc)>> {
        let mut out = Vec::new();
        collect_images(&self.root, "", &mut out)?;
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }
}

fn collect_images(node: &WzNodeArc, path: &str, out: &mut Vec<(String, WzNodeArc)>) -> Result<()> {
    let children: Vec<_> = {
        let read = node.read().expect("poisoned lock");
        match read.object_type {
            WzObjectType::Image(_) | WzObjectType::MsImage(_) => {
                out.push((path.to_owned(), node.clone()));
                return Ok(());
            }
            WzObjectType::File(_) | WzObjectType::MsFile(_) | WzObjectType::Directory(_) => {}
            _ => return Ok(()),
        }
        drop(read);
        parse(node)?;
        children(node)
    };
    for (name, child) in children {
        let child_path = if path.is_empty() {
            name
        } else {
            format!("{path}/{name}")
        };
        collect_images(&child, &child_path, out)?;
    }
    Ok(())
}

pub fn parse(node: &WzNodeArc) -> Result<()> {
    let mut write = node.write().expect("poisoned lock");
    let name = write.name.to_string();
    write
        .parse(node)
        .map_err(|e| anyhow!("parsing {name:?}: {e}"))
}

/// Parse `node` and its descendants down to `depth` levels (0 = all).
pub fn parse_to_depth(node: &WzNodeArc, depth: usize) -> Result<()> {
    parse(node)?;
    if depth == 1 {
        return Ok(());
    }
    for (_, child) in children(node) {
        parse_to_depth(&child, depth.saturating_sub(1))?;
    }
    Ok(())
}

/// Snapshot of a node's children as `(name, node)`, in storage order.
pub fn children(node: &WzNodeArc) -> Vec<(String, WzNodeArc)> {
    node.read()
        .expect("poisoned lock")
        .children
        .iter()
        .map(|(name, child)| (name.to_string(), child.clone()))
        .collect()
}

/// Release a parsed image's children to free memory.
pub fn unparse(node: &WzNodeArc) {
    node.write().expect("poisoned lock").unparse();
}

/// How to render nodes as JSON.
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonOptions {
    /// Hash the payload of sounds and raw data. Needed for change
    /// detection, too slow for interactive browsing.
    pub content_hashes: bool,
    /// Stop descending after this many levels.
    pub max_depth: Option<usize>,
}

/// Canonical JSON for a node and its (already parsed) children.
///
/// Conventions: vectors are `[x, y]`, UOL links are `{"_uol": path}`,
/// canvases carry a `_canvas` object, sounds a `_sound` object.
pub fn node_to_json(node: &WzNodeArc, opts: JsonOptions) -> Value {
    to_json(node, opts, 0)
}

fn to_json(node: &WzNodeArc, opts: JsonOptions, depth: usize) -> Value {
    let read = node.read().expect("poisoned lock");
    let mut obj = Map::new();
    match &read.object_type {
        WzObjectType::Value(v) => return value_to_json(v, opts),
        WzObjectType::Property(WzSubProperty::PNG(png)) => {
            obj.insert(
                "_canvas".into(),
                json!({ "w": png.width, "h": png.height, "format": format!("{:?}", png.format()) }),
            );
        }
        WzObjectType::Property(WzSubProperty::Sound(sound)) => {
            let mut s =
                json!({ "duration": sound.duration, "type": format!("{:?}", sound.sound_type) });
            if opts.content_hashes {
                s["hash"] = hash_bytes(&sound.get_buffer()).into();
            }
            obj.insert("_sound".into(), s);
        }
        WzObjectType::Property(WzSubProperty::Convex) => {
            obj.insert("_convex".into(), true.into());
        }
        _ => {}
    }

    if opts.max_depth.is_some_and(|max| depth >= max) {
        if !read.children.is_empty() {
            obj.insert("_children".into(), read.children.len().into());
        }
        return Value::Object(obj);
    }
    for (name, child) in &read.children {
        obj.insert(name.to_string(), to_json(child, opts, depth + 1));
    }
    Value::Object(obj)
}

fn value_to_json(v: &WzValue, opts: JsonOptions) -> Value {
    match v {
        WzValue::Null => Value::Null,
        WzValue::Short(n) => (*n).into(),
        WzValue::Int(n) => (*n).into(),
        WzValue::Long(n) => (*n).into(),
        WzValue::Float(f) => float(f64::from(*f)),
        WzValue::Double(f) => float(*f),
        WzValue::Vector(vec) => json!([vec.0, vec.1]),
        WzValue::String(s) => s.get_string().unwrap_or_default().into(),
        WzValue::ParsedString(s) => s.clone().into(),
        WzValue::UOL(s) => json!({ "_uol": s.get_string().unwrap_or_default() }),
        WzValue::Lua(lua) => json!({ "_lua": lua.extract_lua().unwrap_or_default() }),
        WzValue::RawData(raw) => blob_json("_raw", raw.get_buffer(), opts),
        WzValue::Video(video) => blob_json("_video", video.get_buffer(), opts),
    }
}

fn blob_json(tag: &str, bytes: &[u8], opts: JsonOptions) -> Value {
    let mut inner = json!({ "len": bytes.len() });
    if opts.content_hashes {
        inner["hash"] = hash_bytes(bytes).into();
    }
    json!({ tag: inner })
}

fn float(f: f64) -> Value {
    Number::from_f64(f).map_or(Value::Null, Value::Number)
}

/// Child values that name an entry, in order of preference.
const NAME_KEYS: &[&str] = &["name", "mapName", "streetName", "bookName"];

/// Human-readable name of an entry, e.g. `"Sword"` for
/// `String/Eqp.img/.../1302000`, taken from its `name`-like child.
pub fn display_name(node: &WzNodeArc) -> Option<String> {
    let read = node.read().expect("poisoned lock");
    NAME_KEYS.iter().find_map(|key| {
        let child = read.children.get(*key)?;
        match &child.read().expect("poisoned lock").object_type {
            WzObjectType::Value(WzValue::String(s)) => s.get_string().ok(),
            WzObjectType::Value(WzValue::ParsedString(s)) => Some(s.clone()),
            _ => None,
        }
    })
}

/// Order node names so `2` sorts before `10`, as WZ ids should.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y).then_with(|| a.cmp(b)),
        _ => a.cmp(b),
    }
}

/// Short type name for listings.
pub fn type_name(node: &WzNodeArc) -> &'static str {
    let read = node.read().expect("poisoned lock");
    match &read.object_type {
        WzObjectType::File(_) | WzObjectType::MsFile(_) => "file",
        WzObjectType::Directory(_) => "dir",
        WzObjectType::Image(_) | WzObjectType::MsImage(_) => "img",
        WzObjectType::Property(WzSubProperty::PNG(_)) => "canvas",
        WzObjectType::Property(WzSubProperty::Sound(_)) => "sound",
        WzObjectType::Property(WzSubProperty::Convex) => "convex",
        WzObjectType::Property(WzSubProperty::Property) => "prop",
        WzObjectType::Value(v) => match v {
            WzValue::Null => "null",
            WzValue::Short(_) | WzValue::Int(_) | WzValue::Long(_) => "int",
            WzValue::Float(_) | WzValue::Double(_) => "float",
            WzValue::Vector(_) => "vector",
            WzValue::String(_) | WzValue::ParsedString(_) => "string",
            WzValue::UOL(_) => "uol",
            WzValue::Lua(_) => "lua",
            WzValue::RawData(_) => "raw",
            WzValue::Video(_) => "video",
        },
    }
}

/// Decode a canvas node to an image.
pub fn canvas_image(node: &WzNodeArc) -> Result<image::DynamicImage> {
    wz_reader::property::png::get_image(node).map_err(|e| anyhow!("decoding canvas: {e}"))
}

/// Whether `node` holds a value, as opposed to a container.
pub fn is_value(node: &WzNodeArc) -> bool {
    node.read().expect("poisoned lock").try_as_value().is_some()
}
