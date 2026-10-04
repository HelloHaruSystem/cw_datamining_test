//! Command line definitions.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "datamine", version, about = "MS patch datamining")]
pub struct Cli {
    /// Store directory (database + snapshots).
    #[arg(long, global = true, env = "DATAMINE_STORE", default_value = "store")]
    pub store: PathBuf,

    /// More logging (-v info, -vv debug).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

/// Versions are selected by label, `#<id>`, `latest`, `previous` or `baseline`.
#[derive(Subcommand)]
pub enum Command {
    /// Copy a client into the store as a new version, then extract it.
    /// The source client is only read, never modified or parsed in place.
    Import {
        /// Client directory (the one containing Data/).
        client: PathBuf,
        /// Unique name for this version, e.g. v262-prerelease.
        #[arg(long)]
        label: String,
        /// Launcher patchdata/ directory; records the build manifest.
        #[arg(long)]
        patchdata: Option<PathBuf>,
        /// Release channel, e.g. prerelease, live, test.
        #[arg(long, default_value = "live")]
        channel: String,
        #[arg(long)]
        note: Option<String>,
        /// Make this version the baseline for diffs.
        #[arg(long)]
        baseline: bool,
        /// Only copy; run `datamine extract` later.
        #[arg(long)]
        no_extract: bool,
    },

    /// List imported versions.
    Versions,

    /// Delete a version and its snapshot. Without --yes, only shows what
    /// would be deleted.
    Remove {
        version: String,
        #[arg(long)]
        yes: bool,
    },

    /// Free disk space used by files no remaining version references.
    Gc,

    /// Show or set the baseline version that diffs compare against.
    Baseline { version: Option<String> },

    /// Show what has been extracted for a version.
    Info {
        #[arg(default_value = "latest")]
        version: String,
    },

    /// List available extractors.
    Extractors,

    /// (Re)run extractors on a version.
    Extract {
        version: String,
        /// Only run these extractors (repeatable).
        #[arg(long)]
        only: Vec<String>,
    },

    /// Show what changed between two versions.
    Diff {
        /// Old side; defaults to the baseline.
        #[arg(long, default_value = "baseline")]
        from: String,
        /// New side.
        #[arg(long, default_value = "latest")]
        to: String,
        /// Only kinds matching this (prefix, e.g. `string` or `string/Eqp`).
        #[arg(long)]
        kind: Option<String>,
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
        /// Write to a file instead of stdout.
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Max changes listed per kind in text/markdown (0 = all).
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },

    /// Show how a record changed across all versions.
    History {
        /// Record key or id, e.g. 1302000 or Eqp/Weapon/1302000.
        key: String,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        json: bool,
    },

    /// Search record keys and data in a version.
    Search {
        text: String,
        #[arg(long, default_value = "latest")]
        version: String,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },

    /// List the children of a WZ node, e.g. `String/Eqp.img/Eqp`.
    Ls {
        version: String,
        #[arg(default_value = "")]
        path: String,
    },

    /// Print a WZ node as JSON.
    Show {
        version: String,
        path: String,
        /// Levels to expand (0 = unlimited).
        #[arg(long, default_value_t = 3)]
        depth: usize,
    },

    /// Start the web viewer.
    Serve {
        /// Address to listen on. Use 0.0.0.0:<port> to expose it on the network.
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: std::net::SocketAddr,
    },

    /// Save a canvas node as PNG.
    ExportImage {
        version: String,
        path: String,
        #[arg(long, short)]
        out: PathBuf,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Text,
    Markdown,
    Json,
}
