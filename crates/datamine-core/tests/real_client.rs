//! End-to-end test against a real client. Opt-in because it needs game
//! files that are not in the repository:
//!
//! ```sh
//! DATAMINE_TEST_CLIENT=cw_test_2/appdata cargo test -p datamine-core -- --ignored
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use datamine_core::import::{ImportOptions, import};
use datamine_core::{Store, extract};
use walkdir::WalkDir;

fn client_dir() -> PathBuf {
    std::env::var_os("DATAMINE_TEST_CLIENT")
        .expect("set DATAMINE_TEST_CLIENT to a client directory")
        .into()
}

/// Path -> (size, mtime, blake3) for every file under `dir`.
fn fingerprint(dir: &Path) -> BTreeMap<PathBuf, (u64, std::time::SystemTime, String)> {
    WalkDir::new(dir)
        .into_iter()
        .map(Result::unwrap)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            let meta = e.metadata().unwrap();
            let hash = blake3::hash(&std::fs::read(e.path()).unwrap())
                .to_hex()
                .to_string();
            (e.into_path(), (meta.len(), meta.modified().unwrap(), hash))
        })
        .collect()
}

#[test]
#[ignore = "needs DATAMINE_TEST_CLIENT"]
fn import_and_extract_leaves_source_untouched() {
    let client = client_dir();
    let before = fingerprint(&client);

    let tmp = tempfile::tempdir().unwrap();
    let mut store = Store::open(tmp.path().join("store")).unwrap();
    let report = import(
        &mut store,
        &ImportOptions {
            client_dir: client.clone(),
            patchdata_dir: None,
            label: "it".into(),
            channel: "test".into(),
            note: None,
        },
    )
    .unwrap();
    let summaries = extract::run(&mut store, &report.version, &[]).unwrap();

    for s in &summaries {
        assert!(s.records > 0, "{} produced no records", s.extractor);
    }
    assert_eq!(before, fingerprint(&client), "source client was modified");
}
