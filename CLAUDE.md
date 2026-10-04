MS datamining tool in Rust. Rules:

- Never read or parse a client install directly. Only `import` touches it (read-only copy); everything else uses `store/snapshots/<label>/`.
- Never commit game data (`cw_test_2/`, `store/`).
- SQL only in `crates/datamine-core/src/db/`, portable (Postgres planned), no absolute paths. Schema changes = new migration file, never edit old ones.
- Logic lives in `datamine-core`; `datamine-cli` only parses args and renders.
- Before finishing: `cargo fmt --all && cargo clippy --all-targets && cargo test`
