use clipboardx_model::StorageError;
use rusqlite::Connection;

const HISTORY_SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS schema_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS groups (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL UNIQUE,
    color       TEXT,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS clip_items (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    content_hash     TEXT NOT NULL UNIQUE,
    kind             TEXT NOT NULL,
    detected_subtype TEXT NOT NULL DEFAULT 'plain',
    detected_lang    TEXT,
    preview_text     TEXT NOT NULL,
    search_text      TEXT NOT NULL,
    pinyin_index     TEXT NOT NULL DEFAULT '',
    ocr_text         TEXT NOT NULL DEFAULT '',
    alias            TEXT NOT NULL DEFAULT '',
    payload_inline   TEXT,
    blob_ref         TEXT,
    thumb_ref        TEXT,
    meta_json        TEXT NOT NULL DEFAULT '{}',
    source_app_name  TEXT,
    source_app_id    TEXT,
    is_pinned        INTEGER NOT NULL DEFAULT 0,
    group_id         INTEGER REFERENCES groups(id) ON DELETE SET NULL,
    use_count        INTEGER NOT NULL DEFAULT 1,
    byte_size        INTEGER NOT NULL DEFAULT 0,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_items_pinned_updated ON clip_items(is_pinned DESC, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_items_group_updated  ON clip_items(group_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_items_kind           ON clip_items(kind, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_items_source         ON clip_items(source_app_id);

CREATE VIRTUAL TABLE IF NOT EXISTS clip_items_fts USING fts5(
    search_text,
    alias,
    pinyin_index,
    ocr_text,
    source_app_name,
    content='clip_items',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS clip_items_ai AFTER INSERT ON clip_items BEGIN
    INSERT INTO clip_items_fts(rowid, search_text, alias, pinyin_index, ocr_text, source_app_name)
    VALUES (new.id, new.search_text, new.alias, new.pinyin_index, new.ocr_text, COALESCE(new.source_app_name, ''));
END;

CREATE TRIGGER IF NOT EXISTS clip_items_ad AFTER DELETE ON clip_items BEGIN
    INSERT INTO clip_items_fts(clip_items_fts, rowid, search_text, alias, pinyin_index, ocr_text, source_app_name)
    VALUES ('delete', old.id, old.search_text, old.alias, old.pinyin_index, old.ocr_text, COALESCE(old.source_app_name, ''));
END;

CREATE TRIGGER IF NOT EXISTS clip_items_au AFTER UPDATE ON clip_items BEGIN
    INSERT INTO clip_items_fts(clip_items_fts, rowid, search_text, alias, pinyin_index, ocr_text, source_app_name)
    VALUES ('delete', old.id, old.search_text, old.alias, old.pinyin_index, old.ocr_text, COALESCE(old.source_app_name, ''));
    INSERT INTO clip_items_fts(rowid, search_text, alias, pinyin_index, ocr_text, source_app_name)
    VALUES (new.id, new.search_text, new.alias, new.pinyin_index, new.ocr_text, COALESCE(new.source_app_name, ''));
END;

INSERT OR REPLACE INTO schema_meta(key, value) VALUES ('version', '1');
"#;

const VAULT_SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS vault_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS vault_entries (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    title        TEXT NOT NULL,
    username     TEXT NOT NULL,
    password     TEXT NOT NULL,
    note         TEXT NOT NULL DEFAULT '',
    auto_submit  INTEGER NOT NULL DEFAULT 1,
    pinyin_index TEXT NOT NULL DEFAULT '',
    updated_at   INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_vault_updated ON vault_entries(updated_at DESC);

INSERT OR REPLACE INTO vault_meta(key, value) VALUES ('version', '1');
"#;

/// Applies initial schema migrations for `history.db`.
pub fn migrate_history_db(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(HISTORY_SCHEMA_V1)
        .map_err(|err| StorageError::Database(format!("history migration failed: {err}")))
}

/// Applies initial schema migrations for `vault.db`.
pub fn migrate_vault_db(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(VAULT_SCHEMA_V1)
        .map_err(|err| StorageError::Database(format!("vault migration failed: {err}")))
}
