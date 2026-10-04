//! Per-request site context: which version is being viewed and how to link
//! to other pages while staying on it.
//!
//! Pages show the newest version unless `?v=<label>` pins another one.
//! Links made with [`Site::link`] carry the pin along.

use datamine_core::db::Version;
use serde::Deserialize;

use crate::error::AppError;
use crate::state::Ctx;
use crate::views::encode_query;

#[derive(Debug, Deserialize, Default)]
pub struct VersionParam {
    pub v: Option<String>,
}

pub struct Site {
    pub versions: Vec<Version>,
    pub version: Version,
    /// True when the URL picked the version (as opposed to "newest").
    pub pinned: bool,
    /// Path of the current page, for the version switcher.
    pub path: String,
}

impl Site {
    pub fn load(ctx: &Ctx, param: Option<&str>, path: &str) -> Result<Self, AppError> {
        let versions = ctx.store.db().versions()?;
        let param = param.filter(|v| !v.is_empty());
        let version = match param {
            Some(sel) => ctx.version(sel)?,
            None => versions.last().cloned().ok_or(AppError::NoVersions)?,
        };
        let pinned = param.is_some() && versions.last().is_some_and(|l| l.id != version.id);
        Ok(Self {
            versions,
            version,
            pinned,
            path: path.to_owned(),
        })
    }

    /// `path` on the current version.
    pub fn link(&self, path: &str) -> String {
        if !self.pinned {
            return path.to_owned();
        }
        let sep = if path.contains('?') { '&' } else { '?' };
        format!("{path}{sep}v={}", encode_query(&self.version.label))
    }

    /// Raw-data URL (`/v/<label>/<route>/<path>`) for this version.
    pub fn raw(&self, route: &str, wz_path: &str) -> String {
        crate::views::raw_url(&self.version, route, wz_path)
    }

    pub fn is_latest(&self) -> bool {
        self.versions
            .last()
            .is_some_and(|l| l.id == self.version.id)
    }
}
