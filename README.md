# datamine

MS patch datamining in Rust. Each client build is imported as a version, and
you can diff versions, see a record's history, and browse any version in full.

## Usage

```sh
cargo build --release
alias datamine=./target/release/datamine

datamine import <client-dir> --patchdata <patchdata-dir> --label <name>
datamine versions
datamine diff                          # baseline -> latest
datamine diff --from previous --format markdown -o changes.md
datamine history 1302000
datamine search "Sword" --kind string/Eqp
datamine ls latest String/Eqp.img
datamine show latest String/Skill.img --depth 2
datamine export-image latest Item/Consume/0200.img/02000000/info/icon -o icon.png
```

Versions are picked by label, `#<id>`, `latest`, `previous` or `baseline`.
Run `datamine --help` for everything else.

`import` copies the client into `store/` first. Nothing ever reads or parses
the original install.

## Layout

```text
crates/datamine-core   all logic (import, db, wz parsing, extractors, diff)
crates/datamine-cli    argument parsing and output only
store/                 datamine.db + deduplicated snapshots (git-ignored)
```

Everything tracked is a record `(kind, key, hash, data)`. To track something
new, add an extractor in `crates/datamine-core/src/extract/` and register it.

## Tests

```sh
cargo test
DATAMINE_TEST_CLIENT=$PWD/cw_test_2/appdata cargo test --release -- --include-ignored
```
