//! Core of the MS datamine tool.
//!
//! Pipeline: [`import`] copies a client into the [`Store`] as an immutable
//! snapshot, [`extract`] parses the snapshot into [`record::Record`]s in
//! the database, and [`diff`] compares versions and tracks history.
//! [`wz`] gives direct access to any node of any snapshot for browsing.

pub mod catalog;
pub mod db;
pub mod diff;
pub mod extract;
pub mod facets;
pub mod import;
pub mod jobs;
pub mod objects;
pub mod patchdata;
pub mod record;
pub mod sp;
pub mod store;
pub mod textdiff;
pub mod wz;

pub use store::Store;
