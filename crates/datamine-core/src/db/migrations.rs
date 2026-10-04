//! Forward-only schema migrations, tracked with SQLite's `user_version`.
//!
//! To change the schema, add a new `migrations/sqlite/NNNN_name.sql` file and
//! append it to [`MIGRATIONS`]. Never edit a migration that has shipped.

use anyhow::{Context, Result, bail};
use rusqlite::Connection;

const MIGRATIONS: &[(&str, &str)] = &[(
    "0001_init",
    include_str!("../../migrations/sqlite/0001_init.sql"),
)];

pub fn run(conn: &mut Connection) -> Result<()> {
    let current: usize = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current > MIGRATIONS.len() {
        bail!(
            "database schema version {current} is newer than this build supports ({}); \
             update datamine",
            MIGRATIONS.len()
        );
    }
    for (i, (name, sql)) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("applying migration {name}"))?;
        tx.pragma_update(None, "user_version", i + 1)?;
        tx.commit()?;
        tracing::info!(migration = name, "applied database migration");
    }
    Ok(())
}
