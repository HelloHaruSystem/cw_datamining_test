//! The unit of change tracking.
//!
//! Every piece of data we compare between patches (a file, an `.img`, a
//! String.wz entry, ...) is flattened into a [`Record`]. Diffing and history
//! then work the same way for every kind of data.

use serde::Serialize;
use serde_json::Value;

/// Hex-encoded blake3 hash.
pub type Hash = String;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Record {
    /// What this is, e.g. `"file"`, `"img"`, `"string/Eqp"`.
    pub kind: String,
    /// Identity within `kind`. Must be stable across patches.
    pub key: String,
    /// Change detector: equal hashes mean "unchanged".
    pub hash: Hash,
    /// Optional human-readable payload shown in diffs.
    pub data: Option<Value>,
}

impl Record {
    /// A record whose identity hash is the hash of its payload.
    pub fn from_data(kind: impl Into<String>, key: impl Into<String>, data: Value) -> Self {
        Self {
            kind: kind.into(),
            key: key.into(),
            hash: hash_json(&data),
            data: Some(data),
        }
    }

    /// A record tracked only by hash, with no stored payload.
    pub fn from_hash(kind: impl Into<String>, key: impl Into<String>, hash: Hash) -> Self {
        Self {
            kind: kind.into(),
            key: key.into(),
            hash,
            data: None,
        }
    }
}

/// Hash a JSON value. `serde_json::Map` keeps keys sorted, so this is
/// canonical as long as callers don't enable `preserve_order`.
pub fn hash_json(value: &Value) -> Hash {
    hash_bytes(value.to_string().as_bytes())
}

pub fn hash_bytes(bytes: &[u8]) -> Hash {
    blake3::hash(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_hash_ignores_key_order() {
        let a: Value = serde_json::from_str(r#"{"a":1,"b":2}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"b":2,"a":1}"#).unwrap();
        assert_eq!(hash_json(&a), hash_json(&b));
    }

    #[test]
    fn from_data_hashes_payload() {
        let r = Record::from_data("string/Eqp", "1302000", json!({"name": "Sword"}));
        assert_eq!(r.hash, hash_json(&json!({"name": "Sword"})));
    }
}
