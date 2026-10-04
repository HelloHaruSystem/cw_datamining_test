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
datamine diff --category skills --job warrior --data-only --hide-text
datamine skills --job fighter          # also: datamine jobs, datamine skill <id>
datamine sp --job crusader --level 75 --build 1000000:16,1001003:20
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

## Web viewer

```sh
datamine serve                     # http://127.0.0.1:8080
datamine serve --addr 0.0.0.0:8080 # reachable from other machines
```

Opens on the newest version (switch versions in the header). Pages:

- **Skills**: every job, skill details with per-level stats and history
- **Skill builder**: SP planning per job and level, with shareable links
- **Patch diff**: changes between versions, filterable by category, job,
  added/removed/changed, and hiding text-only rewording
- **Search**, **Raw data** (full WZ tree with images) and **Versions**

Works on phones and desktops, in light or dark mode. Everything except the
skill builder works without JavaScript, and every page has a CLI equivalent.

## Layout

```text
crates/datamine-core   all logic (import, db, wz parsing, extractors, diff,
                       facets, jobs, SP rules)
crates/datamine-cli    argument parsing and output only
crates/datamine-web    web viewer (axum + maud), rendering only
store/                 datamine.db + deduplicated snapshots (git-ignored)
```

Everything tracked is a record `(kind, key, hash, data)`. To track something
new, add an extractor in `crates/datamine-core/src/extract/` and register it.

## Tests

```sh
cargo test
DATAMINE_TEST_CLIENT=$PWD/cw_test_2/appdata cargo test --release -- --include-ignored
```
