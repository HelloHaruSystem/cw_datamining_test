//! Shared server state.
//!
//! The store's SQLite connection is not `Sync`, so it sits behind a mutex;
//! all store and WZ work runs on the blocking pool via [`AppState::run`].
//! Opened WZ trees are cached per version since snapshots never change.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use datamine_core::Store;
use datamine_core::db::{Version, VersionId};
use datamine_core::wz::WzTree;

use crate::error::AppError;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    store: Mutex<Store>,
    trees: Mutex<HashMap<VersionId, Arc<WzTree>>>,
}

impl AppState {
    pub fn open(store_root: &Path) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(Inner {
                store: Mutex::new(Store::open(store_root)?),
                trees: Mutex::new(HashMap::new()),
            }),
        })
    }

    /// Run `f` with the store on the blocking thread pool.
    pub async fn run<T, F>(&self, f: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&Ctx) -> Result<T, AppError> + Send + 'static,
    {
        let state = self.clone();
        tokio::task::spawn_blocking(move || {
            let store = state.inner.store.lock().expect("poisoned store lock");
            f(&Ctx {
                store: &store,
                state: &state,
            })
        })
        .await
        .map_err(|e| AppError::Internal(e.into()))?
    }
}

/// What handlers get inside [`AppState::run`].
pub struct Ctx<'a> {
    pub store: &'a Store,
    state: &'a AppState,
}

impl Ctx<'_> {
    /// Resolve a version selector, as 404 if it doesn't exist.
    pub fn version(&self, selector: &str) -> Result<Version, AppError> {
        self.store.resolve(selector).map_err(AppError::NotFound)
    }

    /// The (cached) WZ tree of a version's snapshot.
    pub fn tree(&self, version: &Version) -> Result<Arc<WzTree>, AppError> {
        let mut trees = self.state.inner.trees.lock().expect("poisoned tree cache");
        if let Some(tree) = trees.get(&version.id) {
            return Ok(tree.clone());
        }
        let tree = Arc::new(WzTree::open(&self.store.snapshot_dir(version))?);
        trees.insert(version.id, tree.clone());
        Ok(tree)
    }
}
