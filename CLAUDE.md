MS datamining tool in Rust. Rules:

- Never read or parse a client install directly. Only `import` touches it (read-only copy); everything else uses `store/snapshots/<label>/`.
- Never commit game data (`cw_test_2/`, `store/`).
- SQL only in `crates/datamine-core/src/db/`, portable (Postgres planned), no absolute paths. Schema changes = new migration file, never edit old ones.
- Logic lives in `datamine-core`; `datamine-cli` and `datamine-web` only call it and render.
- CLI parity: anything the web viewer can show must also be reachable from the CLI.
- Web UI: responsive (phone to desktop), light + dark mode, keyboard accessible.
- One branch per feature; never commit features to `master`.
- Before finishing: `cargo fmt --all && cargo clippy --all-targets && cargo test`
