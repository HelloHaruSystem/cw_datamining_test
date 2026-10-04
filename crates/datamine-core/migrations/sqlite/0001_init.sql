-- One row per imported client snapshot. `id` order is the import order and
-- is what "previous"/"latest" are resolved against.
CREATE TABLE versions (
    id            INTEGER PRIMARY KEY,
    label         TEXT    NOT NULL UNIQUE,
    channel       TEXT    NOT NULL,
    imported_at   TEXT    NOT NULL,
    source_path   TEXT    NOT NULL,
    manifest_hash TEXT,
    build_time    TEXT,
    note          TEXT
);

-- Small key/value settings, e.g. which version is the baseline.
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;

-- Content-addressed JSON payloads, shared between versions so an unchanged
-- record costs one row in `records` and nothing else.
CREATE TABLE blobs (
    hash TEXT PRIMARY KEY,
    data TEXT NOT NULL
) WITHOUT ROWID;

-- Everything we track is a record: a file, an .img, a String.wz entry, ...
--   source: the extractor that produced it (used to replace on re-extract)
--   kind:   what it is, e.g. "file", "img", "string/Eqp"
--   key:    identity within the kind, stable across patches
--   hash:   change detector; same hash = unchanged
CREATE TABLE records (
    version_id INTEGER NOT NULL REFERENCES versions(id) ON DELETE CASCADE,
    source     TEXT    NOT NULL,
    kind       TEXT    NOT NULL,
    key        TEXT    NOT NULL,
    hash       TEXT    NOT NULL,
    blob_hash  TEXT    REFERENCES blobs(hash),
    PRIMARY KEY (version_id, kind, key)
) WITHOUT ROWID;

CREATE INDEX records_by_key    ON records (kind, key, version_id);
CREATE INDEX records_by_source ON records (version_id, source);

-- Which extractors have completed for a version.
CREATE TABLE extractions (
    version_id   INTEGER NOT NULL REFERENCES versions(id) ON DELETE CASCADE,
    extractor    TEXT    NOT NULL,
    record_count INTEGER NOT NULL,
    finished_at  TEXT    NOT NULL,
    PRIMARY KEY (version_id, extractor)
) WITHOUT ROWID;
