use std::path::Path;
use std::sync::{Arc, Mutex};

use clipboardx_model::{
    ClipItem, ClipItemId, ClipKind, ClipSummary, FilterTab, Group, GroupId, RetentionPolicy,
    SmartSubtype, StorageError,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::crypto::BlobStore;
use crate::migrations::migrate_history_db;

/// Input record produced by `clipboardx-core` before persistence.
#[derive(Debug, Clone)]
pub struct NewClipRecord {
    pub content_hash: String,
    pub kind: ClipKind,
    pub subtype: SmartSubtype,
    pub detected_lang: Option<String>,
    pub preview_text: String,
    pub search_text: String,
    pub pinyin_index: String,
    pub payload_inline: Option<String>,
    pub blob_ref: Option<String>,
    pub thumb_ref: Option<String>,
    pub meta_json: String,
    pub source_app_name: Option<String>,
    pub source_app_id: Option<String>,
    pub byte_size: u64,
    pub timestamp_ms: i64,
}

/// Encrypted SQLite (`sqlite3mc` ChaCha20-Poly1305) history repository.
#[derive(Clone)]
pub struct HistoryStore {
    conn: Arc<Mutex<Connection>>,
    blobs: BlobStore,
}

impl HistoryStore {
    /// Opens (or creates) an encrypted `history.db` using `ChaCha20-Poly1305` page encryption.
    pub fn open(db_path: &Path, db_key_hex: &str, blobs: BlobStore) -> Result<Self, StorageError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                StorageError::Database(format!("create db directory failed: {err}"))
            })?;
        }

        let conn = Connection::open(db_path)
            .map_err(|err| StorageError::Database(format!("open sqlite failed: {err}")))?;

        let pragma_sql = format!(
            "PRAGMA cipher = 'chacha20'; PRAGMA key = \"x'{db_key_hex}'\"; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON; PRAGMA cache_size = -1024; PRAGMA temp_store = FILE;"
        );
        conn.execute_batch(&pragma_sql).map_err(|err| {
            StorageError::Crypto(format!("unlock encrypted history.db failed: {err}"))
        })?;

        // Probe sqlite_master immediately to fail fast on wrong key or corrupted header.
        let _: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
            .map_err(|err| StorageError::Crypto(format!("verify history.db key failed: {err}")))?;

        migrate_history_db(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            blobs,
        })
    }

    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    /// Asks SQLite to release non-essential page cache memory during idle standby.
    pub fn shrink_memory(&self) {
        if let Ok(conn) = self.lock_conn() {
            let _ = conn.execute_batch("PRAGMA shrink_memory;");
        }
    }

    /// Inserts a new item or bumps an existing duplicate (`content_hash`) to the top,
    /// preserving `is_pinned`, `group_id`, `alias`, and `ocr_text`.
    pub fn upsert_item(&self, rec: &NewClipRecord) -> Result<ClipItemId, StorageError> {
        let conn = self.lock_conn()?;
        let existing_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM clip_items WHERE content_hash = ?1",
                params![rec.content_hash],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| StorageError::Database(format!("dedup lookup failed: {err}")))?;

        if let Some(id) = existing_id {
            conn.execute(
                "UPDATE clip_items
                 SET updated_at = ?1,
                     use_count = use_count + 1,
                     source_app_name = COALESCE(?2, source_app_name),
                     source_app_id = COALESCE(?3, source_app_id)
                 WHERE id = ?4",
                params![rec.timestamp_ms, rec.source_app_name, rec.source_app_id, id],
            )
            .map_err(|err| StorageError::Database(format!("dedup bump failed: {err}")))?;
            return Ok(ClipItemId(id));
        }

        conn.execute(
            "INSERT INTO clip_items (
                content_hash, kind, detected_subtype, detected_lang,
                preview_text, search_text, pinyin_index,
                payload_inline, blob_ref, thumb_ref, meta_json,
                source_app_name, source_app_id, byte_size, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
            params![
                rec.content_hash,
                rec.kind.as_str(),
                rec.subtype.as_str(),
                rec.detected_lang,
                rec.preview_text,
                rec.search_text,
                rec.pinyin_index,
                rec.payload_inline,
                rec.blob_ref,
                rec.thumb_ref,
                rec.meta_json,
                rec.source_app_name,
                rec.source_app_id,
                rec.byte_size as i64,
                rec.timestamp_ms,
            ],
        )
        .map_err(|err| StorageError::Database(format!("insert clip item failed: {err}")))?;

        Ok(ClipItemId(conn.last_insert_rowid()))
    }

    /// Queries lightweight list summaries filtered by tab, optional group, and optional query.
    pub fn query_summaries(
        &self,
        tab: FilterTab,
        group_id: Option<GroupId>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ClipSummary>, StorageError> {
        if tab == FilterTab::Accounts {
            return Ok(Vec::new());
        }

        let conn = self.lock_conn()?;
        let trimmed = query.trim();

        let mut clauses = Vec::new();
        let mut bind_vals: Vec<rusqlite::types::Value> = Vec::new();

        match tab {
            FilterTab::All | FilterTab::Accounts => {}
            FilterTab::Text => clauses.push("(c.kind = 'text' OR c.kind = 'rich_text')".to_owned()),
            FilterTab::Image => clauses.push("c.kind = 'image'".to_owned()),
            FilterTab::Code => {
                clauses.push("(c.detected_subtype = 'code' OR c.detected_subtype = 'json' OR c.detected_subtype = 'jwt')".to_owned());
            }
            FilterTab::Color => clauses.push("c.detected_subtype = 'color'".to_owned()),
            FilterTab::Files => clauses.push("c.kind = 'files'".to_owned()),
            FilterTab::Pinned => clauses.push("c.is_pinned = 1".to_owned()),
        }

        if let Some(gid) = group_id {
            bind_vals.push(rusqlite::types::Value::Integer(gid.0));
            clauses.push(format!("c.group_id = ?{}", bind_vals.len()));
        }

        if !trimmed.is_empty() {
            let like_pattern = format!("%{trimmed}%");
            bind_vals.push(rusqlite::types::Value::Text(like_pattern));
            let idx = bind_vals.len();
            clauses.push(format!(
                "(c.search_text LIKE ?{idx} OR c.alias LIKE ?{idx} OR c.pinyin_index LIKE ?{idx} OR c.ocr_text LIKE ?{idx} OR COALESCE(c.source_app_name, '') LIKE ?{idx})"
            ));
        }

        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };

        bind_vals.push(rusqlite::types::Value::Integer(limit as i64));
        let limit_idx = bind_vals.len();

        let sql = format!(
            "SELECT c.id, c.content_hash, c.kind, c.detected_subtype, c.detected_lang,
                    c.preview_text, c.alias, c.ocr_text, c.thumb_ref, c.source_app_name,
                    c.is_pinned, c.group_id, c.use_count, c.byte_size, c.meta_json,
                    c.created_at, c.updated_at
             FROM clip_items c
             {where_sql}
             ORDER BY c.is_pinned DESC, c.updated_at DESC
             LIMIT ?{limit_idx}"
        );

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|err| StorageError::Database(format!("prepare query failed: {err}")))?;

        let rows = stmt
            .query_map(rusqlite::params_from_iter(bind_vals), map_summary_row)
            .map_err(|err| StorageError::Database(format!("execute query failed: {err}")))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|err| StorageError::Database(err.to_string()))?);
        }
        Ok(out)
    }

    /// Loads total stored history item count for the filter strip counter.
    pub fn total_count(&self) -> Result<u64, StorageError> {
        let conn = self.lock_conn()?;
        let count: i64 = conn
            .query_row("SELECT count(*) FROM clip_items", [], |r| r.get(0))
            .map_err(|err| StorageError::Database(format!("count query failed: {err}")))?;
        Ok(count.max(0) as u64)
    }

    /// Loads a full item by ID, reading encrypted payload bytes on demand.
    pub fn get_item(&self, id: ClipItemId) -> Result<ClipItem, StorageError> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, content_hash, kind, detected_subtype, detected_lang,
                        preview_text, alias, ocr_text, thumb_ref, source_app_name,
                        is_pinned, group_id, use_count, byte_size, meta_json,
                        created_at, updated_at, search_text, payload_inline, blob_ref
                 FROM clip_items
                 WHERE id = ?1",
            )
            .map_err(|err| StorageError::Database(format!("prepare get_item failed: {err}")))?;

        let item = stmt
            .query_row(params![id.0], |row| {
                let summary = map_summary_row(row)?;
                let full_text: String = row.get(17)?;
                let payload_inline: Option<String> = row.get(18)?;
                let blob_ref: Option<String> = row.get(19)?;
                Ok(ClipItem {
                    summary,
                    full_text,
                    payload_inline,
                    blob_ref,
                })
            })
            .optional()
            .map_err(|err| StorageError::Database(format!("get_item failed: {err}")))?;

        item.ok_or_else(|| StorageError::NotFound(id.to_string()))
    }

    /// Toggles the pinned status of an item and returns the updated state.
    pub fn toggle_pin(&self, id: ClipItemId) -> Result<bool, StorageError> {
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE clip_items SET is_pinned = CASE WHEN is_pinned = 0 THEN 1 ELSE 0 END WHERE id = ?1",
            params![id.0],
        )
        .map_err(|err| StorageError::Database(format!("toggle_pin failed: {err}")))?;

        let pinned: i64 = conn
            .query_row(
                "SELECT is_pinned FROM clip_items WHERE id = ?1",
                params![id.0],
                |r| r.get(0),
            )
            .map_err(|err| StorageError::Database(format!("read pin state failed: {err}")))?;
        Ok(pinned != 0)
    }

    /// Updates an item's searchable alias/note.
    pub fn update_alias(&self, id: ClipItemId, alias: &str) -> Result<(), StorageError> {
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE clip_items SET alias = ?1 WHERE id = ?2",
            params![alias, id.0],
        )
        .map_err(|err| StorageError::Database(format!("update_alias failed: {err}")))?;
        Ok(())
    }

    /// Updates an existing text/rich-text item's content in place, refreshing its subtype,
    /// preview, searchable text, pinyin index, and inline/blob payload. If another record
    /// already owns the resulting `content_hash`, merges cleanly and returns that record's ID.
    pub fn update_text_record(
        &self,
        id: ClipItemId,
        rec: &NewClipRecord,
    ) -> Result<ClipItemId, StorageError> {
        let (old_blob, duplicate_id): (Option<String>, Option<i64>) = {
            let conn = self.lock_conn()?;
            let old_blob: Option<String> = conn
                .query_row(
                    "SELECT blob_ref FROM clip_items WHERE id = ?1",
                    params![id.0],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|err| StorageError::Database(format!("load old blob failed: {err}")))?
                .flatten();

            let duplicate_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM clip_items WHERE content_hash = ?1 AND id != ?2",
                    params![rec.content_hash, id.0],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|err| StorageError::Database(format!("check dup hash failed: {err}")))?;

            if let Some(dup_id) = duplicate_id {
                conn.execute(
                    "UPDATE clip_items SET updated_at = ?1, use_count = use_count + 1 WHERE id = ?2",
                    params![rec.timestamp_ms, dup_id],
                )
                .map_err(|err| StorageError::Database(format!("bump dup item failed: {err}")))?;
                conn.execute("DELETE FROM clip_items WHERE id = ?1", params![id.0])
                    .map_err(|err| {
                        StorageError::Database(format!("remove merged source item failed: {err}"))
                    })?;
            } else {
                conn.execute(
                    "UPDATE clip_items
                     SET content_hash = ?1,
                         kind = ?2,
                         detected_subtype = ?3,
                         detected_lang = ?4,
                         preview_text = ?5,
                         search_text = ?6,
                         pinyin_index = ?7,
                         payload_inline = ?8,
                         blob_ref = ?9,
                         meta_json = ?10,
                         byte_size = ?11,
                         updated_at = ?12
                     WHERE id = ?13",
                    params![
                        rec.content_hash,
                        rec.kind.as_str(),
                        rec.subtype.as_str(),
                        rec.detected_lang,
                        rec.preview_text,
                        rec.search_text,
                        rec.pinyin_index,
                        rec.payload_inline,
                        rec.blob_ref,
                        rec.meta_json,
                        rec.byte_size as i64,
                        rec.timestamp_ms,
                        id.0,
                    ],
                )
                .map_err(|err| {
                    StorageError::Database(format!("update_text_record failed: {err}"))
                })?;
            }
            (old_blob, duplicate_id)
        };

        if let Some(old_ref) = old_blob
            && rec.blob_ref.as_deref() != Some(old_ref.as_str())
        {
            self.gc_blob_if_unreferenced(&old_ref)?;
        }

        Ok(ClipItemId(duplicate_id.unwrap_or(id.0)))
    }

    /// Persists recognized OCR text for an image item, keeping the FTS5 index synchronized.
    pub fn update_ocr_text(&self, id: ClipItemId, ocr_text: &str) -> Result<(), StorageError> {
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE clip_items SET ocr_text = ?1 WHERE id = ?2",
            params![ocr_text.trim(), id.0],
        )
        .map_err(|err| StorageError::Database(format!("update_ocr_text failed: {err}")))?;
        Ok(())
    }

    /// Lists all user-defined groups.
    pub fn list_groups(&self) -> Result<Vec<Group>, StorageError> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare("SELECT id, name, color, sort_order, created_at FROM groups ORDER BY sort_order ASC, id ASC")
            .map_err(|err| StorageError::Database(format!("list_groups prepare failed: {err}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Group {
                    id: GroupId(row.get(0)?),
                    name: row.get(1)?,
                    color: row.get(2)?,
                    sort_order: row.get(3)?,
                    created_at_ms: row.get(4)?,
                })
            })
            .map_err(|err| StorageError::Database(format!("list_groups failed: {err}")))?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|err| StorageError::Database(err.to_string()))?);
        }
        Ok(out)
    }

    /// Creates a new custom group (or returns the existing group ID with the same name).
    pub fn create_group(&self, name: &str, now_ms: i64) -> Result<GroupId, StorageError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(StorageError::Database(
                "group name cannot be empty".to_owned(),
            ));
        }
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO groups (name, created_at) VALUES (?1, ?2)",
            params![trimmed, now_ms],
        )
        .map_err(|err| StorageError::Database(format!("create_group failed: {err}")))?;

        let id: i64 = conn
            .query_row(
                "SELECT id FROM groups WHERE name = ?1",
                params![trimmed],
                |r| r.get(0),
            )
            .map_err(|err| StorageError::Database(format!("lookup group failed: {err}")))?;
        Ok(GroupId(id))
    }

    /// Assigns (or clears when `None`) the custom group for a history item.
    pub fn assign_group(
        &self,
        item_id: ClipItemId,
        group_id: Option<GroupId>,
    ) -> Result<(), StorageError> {
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE clip_items SET group_id = ?1 WHERE id = ?2",
            params![group_id.map(|g| g.0), item_id.0],
        )
        .map_err(|err| StorageError::Database(format!("assign_group failed: {err}")))?;
        Ok(())
    }

    /// Deletes a user group (items remain in history with `group_id = NULL`).
    pub fn delete_group(&self, id: GroupId) -> Result<(), StorageError> {
        let conn = self.lock_conn()?;
        conn.execute("DELETE FROM groups WHERE id = ?1", params![id.0])
            .map_err(|err| StorageError::Database(format!("delete_group failed: {err}")))?;
        Ok(())
    }

    /// Clears all non-pinned history items and garbage-collects unreferenced blobs.
    pub fn clear_unpinned(&self) -> Result<usize, StorageError> {
        let orphaned: Vec<(Option<String>, Option<String>)> = {
            let conn = self.lock_conn()?;
            let mut stmt = conn
                .prepare("SELECT blob_ref, thumb_ref FROM clip_items WHERE is_pinned = 0")
                .map_err(|err| StorageError::Database(format!("clear_unpinned prepare: {err}")))?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|err| StorageError::Database(format!("clear_unpinned query: {err}")))?;
            let mut list = Vec::new();
            for row in rows {
                list.push(row.map_err(|err| StorageError::Database(err.to_string()))?);
            }
            list
        };

        let deleted = {
            let conn = self.lock_conn()?;
            conn.execute("DELETE FROM clip_items WHERE is_pinned = 0", [])
                .map_err(|err| StorageError::Database(format!("clear_unpinned delete: {err}")))?
        };

        for (blob_ref, thumb_ref) in orphaned {
            for hash in [blob_ref, thumb_ref].into_iter().flatten() {
                self.gc_blob_if_unreferenced(&hash)?;
            }
        }
        Ok(deleted)
    }

    /// Exports all stored history items with complete inline payloads and blob references.
    pub fn export_all_items(&self) -> Result<Vec<ClipItem>, StorageError> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, content_hash, kind, detected_subtype, detected_lang,
                        preview_text, alias, ocr_text, thumb_ref, source_app_name,
                        is_pinned, group_id, use_count, byte_size, meta_json,
                        created_at, updated_at, search_text, payload_inline, blob_ref
                 FROM clip_items
                 ORDER BY updated_at ASC",
            )
            .map_err(|err| StorageError::Database(format!("export prepare failed: {err}")))?;

        let rows = stmt
            .query_map([], |row| {
                let summary = map_summary_row(row)?;
                let full_text: String = row.get(17)?;
                let payload_inline: Option<String> = row.get(18)?;
                let blob_ref: Option<String> = row.get(19)?;
                Ok(ClipItem {
                    summary,
                    full_text,
                    payload_inline,
                    blob_ref,
                })
            })
            .map_err(|err| StorageError::Database(format!("export query failed: {err}")))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|err| StorageError::Database(err.to_string()))?);
        }
        Ok(out)
    }

    /// Inserts an archived record idempotently by `content_hash`.
    /// Existing items keep their current alias, pin state, and group assignment while filling blanks.
    /// Returns `Ok(true)` if inserted as a new row, or `Ok(false)` if merged/skipped.
    #[allow(clippy::too_many_arguments)]
    pub fn import_item_idempotent(
        &self,
        rec: &NewClipRecord,
        is_pinned: bool,
        group_id: Option<GroupId>,
        alias: &str,
        ocr_text: &str,
        created_at_ms: i64,
    ) -> Result<bool, StorageError> {
        let conn = self.lock_conn()?;
        let existing: Option<(i64, String, String)> = conn
            .query_row(
                "SELECT id, alias, ocr_text FROM clip_items WHERE content_hash = ?1",
                params![rec.content_hash],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|err| StorageError::Database(format!("import lookup failed: {err}")))?;

        if let Some((id, cur_alias, cur_ocr)) = existing {
            let next_alias = if cur_alias.is_empty() {
                alias
            } else {
                &cur_alias
            };
            let next_ocr = if cur_ocr.is_empty() {
                ocr_text
            } else {
                &cur_ocr
            };
            if next_alias != cur_alias || next_ocr != cur_ocr {
                conn.execute(
                    "UPDATE clip_items SET alias = ?1, ocr_text = ?2 WHERE id = ?3",
                    params![next_alias, next_ocr, id],
                )
                .map_err(|err| StorageError::Database(format!("import merge failed: {err}")))?;
            }
            return Ok(false);
        }

        conn.execute(
            "INSERT INTO clip_items (
                content_hash, kind, detected_subtype, detected_lang,
                preview_text, search_text, pinyin_index, ocr_text, alias,
                payload_inline, blob_ref, thumb_ref, meta_json,
                source_app_name, source_app_id, is_pinned, group_id,
                byte_size, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            params![
                rec.content_hash,
                rec.kind.as_str(),
                rec.subtype.as_str(),
                rec.detected_lang,
                rec.preview_text,
                rec.search_text,
                rec.pinyin_index,
                ocr_text,
                alias,
                rec.payload_inline,
                rec.blob_ref,
                rec.thumb_ref,
                rec.meta_json,
                rec.source_app_name,
                rec.source_app_id,
                i64::from(is_pinned),
                group_id.map(|g| g.0),
                rec.byte_size as i64,
                created_at_ms,
                rec.timestamp_ms,
            ],
        )
        .map_err(|err| StorageError::Database(format!("import insert failed: {err}")))?;

        Ok(true)
    }

    /// Deletes a single item and garbage-collects unreferenced encrypted blobs.
    pub fn delete_item(&self, id: ClipItemId) -> Result<(), StorageError> {
        let orphaned = {
            let conn = self.lock_conn()?;
            let refs: Option<(Option<String>, Option<String>)> = conn
                .query_row(
                    "SELECT blob_ref, thumb_ref FROM clip_items WHERE id = ?1",
                    params![id.0],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(|err| StorageError::Database(format!("read blob refs failed: {err}")))?;

            conn.execute("DELETE FROM clip_items WHERE id = ?1", params![id.0])
                .map_err(|err| StorageError::Database(format!("delete_item failed: {err}")))?;

            refs
        };

        if let Some((blob_ref, thumb_ref)) = orphaned {
            for candidate in [blob_ref, thumb_ref].into_iter().flatten() {
                self.gc_blob_if_unreferenced(&candidate)?;
            }
        }
        Ok(())
    }

    /// Enforces retention limits (`max_days`, `max_items`) while keeping pinned and grouped items exempt.
    pub fn prune_retention(
        &self,
        policy: &RetentionPolicy,
        now_ms: i64,
    ) -> Result<usize, StorageError> {
        let cutoff_ms = now_ms - (policy.max_days as i64) * 86_400_000;
        let candidates: Vec<(i64, Option<String>, Option<String>)> = {
            let conn = self.lock_conn()?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, blob_ref, thumb_ref
                     FROM clip_items
                     WHERE is_pinned = 0 AND group_id IS NULL
                       AND (
                         updated_at < ?1
                         OR id NOT IN (
                           SELECT id FROM clip_items
                           WHERE is_pinned = 0 AND group_id IS NULL
                           ORDER BY updated_at DESC
                           LIMIT ?2
                         )
                       )",
                )
                .map_err(|err| StorageError::Database(format!("prune prepare failed: {err}")))?;

            let rows = stmt
                .query_map(params![cutoff_ms, policy.max_items as i64], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .map_err(|err| StorageError::Database(format!("prune query failed: {err}")))?;

            let mut list = Vec::new();
            for row in rows {
                list.push(row.map_err(|err| StorageError::Database(err.to_string()))?);
            }
            list
        };

        if candidates.is_empty() {
            return Ok(0);
        }

        let deleted = candidates.len();
        {
            let mut conn = self.lock_conn()?;
            let tx = conn
                .transaction()
                .map_err(|err| StorageError::Database(format!("prune tx failed: {err}")))?;
            for (id, _, _) in &candidates {
                tx.execute("DELETE FROM clip_items WHERE id = ?1", params![id])
                    .map_err(|err| StorageError::Database(format!("prune delete failed: {err}")))?;
            }
            tx.commit()
                .map_err(|err| StorageError::Database(format!("prune commit failed: {err}")))?;
        }

        for (_, blob_ref, thumb_ref) in candidates {
            for hash in [blob_ref, thumb_ref].into_iter().flatten() {
                self.gc_blob_if_unreferenced(&hash)?;
            }
        }

        Ok(deleted)
    }

    fn gc_blob_if_unreferenced(&self, hash: &str) -> Result<(), StorageError> {
        let count: i64 = {
            let conn = self.lock_conn()?;
            conn.query_row(
                "SELECT count(*) FROM clip_items WHERE blob_ref = ?1 OR thumb_ref = ?1",
                params![hash],
                |r| r.get(0),
            )
            .map_err(|err| StorageError::Database(format!("gc ref check failed: {err}")))?
        };
        if count == 0 {
            self.blobs.remove_if_exists(hash)?;
        }
        Ok(())
    }

    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StorageError> {
        self.conn
            .lock()
            .map_err(|_| StorageError::Database("history db mutex poisoned".to_owned()))
    }
}

fn map_summary_row(row: &Row<'_>) -> rusqlite::Result<ClipSummary> {
    let kind_str: String = row.get(2)?;
    let subtype_str: String = row.get(3)?;
    let group_id_opt: Option<i64> = row.get(11)?;
    let is_pinned: i64 = row.get(10)?;
    let use_count: i64 = row.get(12)?;
    let byte_size: i64 = row.get(13)?;

    Ok(ClipSummary {
        id: ClipItemId(row.get(0)?),
        content_hash: row.get(1)?,
        kind: ClipKind::parse(&kind_str).unwrap_or(ClipKind::Text),
        subtype: SmartSubtype::parse(&subtype_str),
        detected_lang: row.get(4)?,
        preview_text: row.get(5)?,
        alias: row.get(6)?,
        ocr_text: row.get(7)?,
        thumb_ref: row.get(8)?,
        source_app_name: row.get(9)?,
        is_pinned: is_pinned != 0,
        group_id: group_id_opt.map(GroupId),
        use_count: use_count.max(0) as u32,
        byte_size: byte_size.max(0) as u64,
        meta_json: row.get(14)?,
        created_at_ms: row.get(15)?,
        updated_at_ms: row.get(16)?,
    })
}
