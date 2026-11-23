use std::path::Path;

use std::time::{SystemTime, UNIX_EPOCH};

use clipboardx_model::{
    AppConfig, CapturedClipboardEvent, ClipItem, ClipItemId, ClipKind, ClipSummary,
    ClipboardPayload, ClipboardXError, FilterTab, Group, GroupId, RetentionPolicy,
};
use clipboardx_storage::HistoryStore;
use clipboardx_storage::crypto::sha256_hex;
use clipboardx_storage::db::NewClipRecord;
use serde_json::json;

use crate::inspector::inspect_text;
use crate::pinyin_index::build_pinyin_index;
use crate::search::{filter_and_rank_summaries, parse_query};

const INLINE_TEXT_LIMIT_BYTES: usize = 64 * 1024;
const PREVIEW_TRUNCATE_CHARS: usize = 240;

/// Central application service orchestrating clipboard ingestion, deduplication,
/// query filtering, and payload reconstruction for paste write-back.
#[derive(Clone)]
pub struct ClipboardService {
    store: HistoryStore,
}

impl ClipboardService {
    pub fn new(store: HistoryStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &HistoryStore {
        &self.store
    }

    /// Ingests a captured clipboard event, enforcing size guards, smart inspection,
    /// pinyin indexing, encryption of large blobs, and SHA-256 deduplication.
    pub fn ingest_event(
        &self,
        event: CapturedClipboardEvent,
        config: &AppConfig,
    ) -> Result<Option<ClipItemId>, ClipboardXError> {
        let mut record = match event.payload {
            ClipboardPayload::Text { plain } => {
                if plain.trim().is_empty() {
                    return Ok(None);
                }
                let byte_size = plain.len() as u64;
                if byte_size > config.retention.max_item_bytes {
                    return Ok(None);
                }

                let hash = sha256_hex(plain.as_bytes());
                let inspected = inspect_text(&plain);
                let preview = truncate_preview(&plain);
                let pinyin_idx = build_pinyin_index(&plain);
                let (payload_inline, blob_ref) = if plain.len() <= INLINE_TEXT_LIMIT_BYTES {
                    (Some(plain.clone()), None)
                } else {
                    let href = self.store.blobs().put_bytes(plain.as_bytes())?;
                    (None, Some(href))
                };

                let meta = json!({
                    "chars": plain.chars().count(),
                    "lines": plain.lines().count().max(1),
                    "color_hex": inspected.color_hex,
                });

                NewClipRecord {
                    content_hash: hash,
                    kind: ClipKind::Text,
                    subtype: inspected.subtype,
                    detected_lang: inspected.detected_lang,
                    preview_text: preview,
                    search_text: plain,
                    pinyin_index: pinyin_idx,
                    payload_inline,
                    blob_ref,
                    thumb_ref: None,
                    meta_json: meta.to_string(),
                    source_app_name: event.source_app_name,
                    source_app_id: event.source_app_id,
                    byte_size,
                    timestamp_ms: event.captured_at_ms,
                }
            }
            ClipboardPayload::RichText { plain, html, rtf } => {
                if plain.trim().is_empty() && html.is_none() && rtf.is_none() {
                    return Ok(None);
                }
                let serialized = json!({ "plain": plain, "html": html, "rtf": rtf }).to_string();
                let byte_size = serialized.len() as u64;
                if byte_size > config.retention.max_item_bytes {
                    return Ok(None);
                }

                let hash = sha256_hex(serialized.as_bytes());
                let inspected = inspect_text(&plain);
                let preview = truncate_preview(&plain);
                let pinyin_idx = build_pinyin_index(&plain);

                let (payload_inline, blob_ref) = if serialized.len() <= INLINE_TEXT_LIMIT_BYTES {
                    (Some(serialized), None)
                } else {
                    let href = self.store.blobs().put_bytes(serialized.as_bytes())?;
                    (None, Some(href))
                };

                let meta = json!({
                    "chars": plain.chars().count(),
                    "lines": plain.lines().count().max(1),
                    "has_html": html.is_some(),
                    "has_rtf": rtf.is_some(),
                });

                NewClipRecord {
                    content_hash: hash,
                    kind: ClipKind::RichText,
                    subtype: inspected.subtype,
                    detected_lang: inspected.detected_lang,
                    preview_text: preview,
                    search_text: plain,
                    pinyin_index: pinyin_idx,
                    payload_inline,
                    blob_ref,
                    thumb_ref: None,
                    meta_json: meta.to_string(),
                    source_app_name: event.source_app_name,
                    source_app_id: event.source_app_id,
                    byte_size,
                    timestamp_ms: event.captured_at_ms,
                }
            }
            ClipboardPayload::Image {
                png_bytes,
                width,
                height,
            } => {
                let byte_size = png_bytes.len() as u64;
                if byte_size == 0 || byte_size > config.retention.max_item_bytes {
                    return Ok(None);
                }
                let hash = sha256_hex(&png_bytes);
                let blob_ref = self.store.blobs().put_bytes(&png_bytes)?;
                let thumb_ref = self.store.blobs().put_image_thumbnail(&png_bytes).ok();

                let preview = format!("截图 {width}×{height} · {}", format_bytes(byte_size));
                let meta = json!({ "width": width, "height": height, "format": "PNG" });

                NewClipRecord {
                    content_hash: hash,
                    kind: ClipKind::Image,
                    subtype: clipboardx_model::SmartSubtype::Plain,
                    detected_lang: None,
                    preview_text: preview,
                    search_text: format!("image {width}x{height}"),
                    pinyin_index: String::new(),
                    payload_inline: None,
                    blob_ref: Some(blob_ref),
                    thumb_ref,
                    meta_json: meta.to_string(),
                    source_app_name: event.source_app_name,
                    source_app_id: event.source_app_id,
                    byte_size,
                    timestamp_ms: event.captured_at_ms,
                }
            }
            ClipboardPayload::Files { paths } => {
                if paths.is_empty() {
                    return Ok(None);
                }
                let joined = paths.join("\n");
                let serialized = serde_json::to_string(&paths)
                    .map_err(|err| ClipboardXError::Validation(err.to_string()))?;
                let hash = sha256_hex(serialized.as_bytes());
                let first_name = Path::new(&paths[0])
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&paths[0]);
                let preview = if paths.len() == 1 {
                    first_name.to_owned()
                } else {
                    format!("{} 个文件 · {first_name}", paths.len())
                };
                let pinyin_idx = build_pinyin_index(&joined);
                let meta = json!({ "file_count": paths.len(), "paths": paths });

                NewClipRecord {
                    content_hash: hash,
                    kind: ClipKind::Files,
                    subtype: clipboardx_model::SmartSubtype::Plain,
                    detected_lang: None,
                    preview_text: preview,
                    search_text: joined,
                    pinyin_index: pinyin_idx,
                    payload_inline: Some(serialized),
                    blob_ref: None,
                    thumb_ref: None,
                    meta_json: meta.to_string(),
                    source_app_name: event.source_app_name,
                    source_app_id: event.source_app_id,
                    byte_size: 0,
                    timestamp_ms: event.captured_at_ms,
                }
            }
        };

        // Ensure preview never contains raw newlines that break single-line 40px rows.
        record.preview_text = record.preview_text.replace(['\r', '\n'], " ");
        let id = self.store.upsert_item(&record)?;
        Ok(Some(id))
    }

    pub fn query_summaries(
        &self,
        tab: FilterTab,
        group_id: Option<GroupId>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ClipSummary>, ClipboardXError> {
        let parsed = parse_query(query);
        let effective_tab = if tab == FilterTab::All {
            parsed.type_filter.unwrap_or(FilterTab::All)
        } else {
            tab
        };

        // When regex or app filters are present, pull a broader candidate slice for local evaluation.
        let sql_query = if parsed.regex_pattern.is_some() || parsed.app_filter.is_some() {
            ""
        } else {
            &parsed.free_text
        };

        let candidates = self
            .store
            .query_summaries(effective_tab, group_id, sql_query, limit)?;
        let ranked = filter_and_rank_summaries(candidates, &parsed);
        Ok(ranked.into_iter().take(limit).collect())
    }

    pub fn total_count(&self) -> Result<u64, ClipboardXError> {
        Ok(self.store.total_count()?)
    }

    pub fn get_item(&self, id: ClipItemId) -> Result<ClipItem, ClipboardXError> {
        Ok(self.store.get_item(id)?)
    }

    pub fn toggle_pin(&self, id: ClipItemId) -> Result<bool, ClipboardXError> {
        Ok(self.store.toggle_pin(id)?)
    }

    pub fn update_alias(&self, id: ClipItemId, alias: &str) -> Result<(), ClipboardXError> {
        Ok(self.store.update_alias(id, alias.trim())?)
    }

    /// Updates an existing text item's content in place, recomputing subtype, preview,
    /// pinyin index, and FTS5 searchable content.
    pub fn update_item_text(
        &self,
        id: ClipItemId,
        new_text: &str,
    ) -> Result<ClipItemId, ClipboardXError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let rec = self.build_text_record(new_text, Some("ClipboardX".to_owned()), now_ms)?;
        Ok(self.store.update_text_record(id, &rec)?)
    }

    /// Inserts a newly edited text entry as a fresh history item, optionally preserving
    /// an alias and group assignment.
    pub fn create_text_item(
        &self,
        text: &str,
        alias: &str,
        group_id: Option<GroupId>,
    ) -> Result<ClipItemId, ClipboardXError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let rec = self.build_text_record(text, Some("ClipboardX".to_owned()), now_ms)?;
        let id = self.store.upsert_item(&rec)?;
        if !alias.trim().is_empty() {
            let _ = self.store.update_alias(id, alias.trim());
        }
        if let Some(gid) = group_id {
            let _ = self.store.assign_group(id, Some(gid));
        }
        Ok(id)
    }

    fn build_text_record(
        &self,
        plain: &str,
        source_app_name: Option<String>,
        timestamp_ms: i64,
    ) -> Result<NewClipRecord, ClipboardXError> {
        let hash = sha256_hex(plain.as_bytes());
        let inspected = inspect_text(plain);
        let preview = truncate_preview(plain).replace(['\r', '\n'], " ");
        let pinyin_idx = build_pinyin_index(plain);
        let (payload_inline, blob_ref) = if plain.len() <= INLINE_TEXT_LIMIT_BYTES {
            (Some(plain.to_owned()), None)
        } else {
            let href = self.store.blobs().put_bytes(plain.as_bytes())?;
            (None, Some(href))
        };
        let meta = json!({
            "chars": plain.chars().count(),
            "lines": plain.lines().count().max(1),
            "color_hex": inspected.color_hex,
        });
        Ok(NewClipRecord {
            content_hash: hash,
            kind: ClipKind::Text,
            subtype: inspected.subtype,
            detected_lang: inspected.detected_lang,
            preview_text: preview,
            search_text: plain.to_owned(),
            pinyin_index: pinyin_idx,
            payload_inline,
            blob_ref,
            thumb_ref: None,
            meta_json: meta.to_string(),
            source_app_name,
            source_app_id: None,
            byte_size: plain.len() as u64,
            timestamp_ms,
        })
    }

    pub fn update_ocr_text(&self, id: ClipItemId, ocr_text: &str) -> Result<(), ClipboardXError> {
        Ok(self.store.update_ocr_text(id, ocr_text)?)
    }

    pub fn delete_item(&self, id: ClipItemId) -> Result<(), ClipboardXError> {
        Ok(self.store.delete_item(id)?)
    }

    pub fn list_groups(&self) -> Result<Vec<Group>, ClipboardXError> {
        Ok(self.store.list_groups()?)
    }

    pub fn create_group(&self, name: &str) -> Result<GroupId, ClipboardXError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Ok(self.store.create_group(name, now_ms)?)
    }

    pub fn delete_group(&self, id: GroupId) -> Result<(), ClipboardXError> {
        Ok(self.store.delete_group(id)?)
    }

    pub fn assign_group(
        &self,
        item_id: ClipItemId,
        group_id: Option<GroupId>,
    ) -> Result<(), ClipboardXError> {
        Ok(self.store.assign_group(item_id, group_id)?)
    }

    pub fn prune_retention(&self, policy: &RetentionPolicy) -> Result<usize, ClipboardXError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Ok(self.store.prune_retention(policy, now_ms)?)
    }

    pub fn clear_unpinned(&self) -> Result<usize, ClipboardXError> {
        Ok(self.store.clear_unpinned()?)
    }

    /// Reconstructs a `ClipboardPayload` ready for OS clipboard write-back.
    pub fn load_payload_for_paste(
        &self,
        id: ClipItemId,
        plain_text_only: bool,
    ) -> Result<ClipboardPayload, ClipboardXError> {
        let item = self.store.get_item(id)?;

        if plain_text_only {
            return Ok(ClipboardPayload::Text {
                plain: item.full_text,
            });
        }

        match item.summary.kind {
            ClipKind::Text => {
                let text = match (item.payload_inline, item.blob_ref) {
                    (Some(inline), _) => inline,
                    (None, Some(blob)) => {
                        let bytes = self.store.blobs().get_bytes(&blob)?;
                        String::from_utf8_lossy(&bytes).into_owned()
                    }
                    (None, None) => item.full_text,
                };
                Ok(ClipboardPayload::Text { plain: text })
            }
            ClipKind::RichText => {
                let raw_json = match (item.payload_inline, item.blob_ref) {
                    (Some(inline), _) => inline,
                    (None, Some(blob)) => {
                        let bytes = self.store.blobs().get_bytes(&blob)?;
                        String::from_utf8_lossy(&bytes).into_owned()
                    }
                    (None, None) => {
                        return Ok(ClipboardPayload::Text {
                            plain: item.full_text,
                        });
                    }
                };

                let val: serde_json::Value = serde_json::from_str(&raw_json)
                    .map_err(|err| ClipboardXError::Validation(err.to_string()))?;
                Ok(ClipboardPayload::RichText {
                    plain: val["plain"].as_str().unwrap_or(&item.full_text).to_owned(),
                    html: val["html"].as_str().map(str::to_owned),
                    rtf: val["rtf"].as_str().map(str::to_owned),
                })
            }
            ClipKind::Image => {
                let blob = item.blob_ref.ok_or_else(|| {
                    ClipboardXError::Validation("missing image blob_ref".to_owned())
                })?;
                let png_bytes = self.store.blobs().get_bytes(&blob)?;
                let meta: serde_json::Value =
                    serde_json::from_str(&item.summary.meta_json).unwrap_or_default();
                let width = meta["width"].as_u64().unwrap_or(0) as u32;
                let height = meta["height"].as_u64().unwrap_or(0) as u32;
                Ok(ClipboardPayload::Image {
                    png_bytes,
                    width,
                    height,
                })
            }
            ClipKind::Files => {
                let raw = item
                    .payload_inline
                    .ok_or_else(|| ClipboardXError::Validation("missing file paths".to_owned()))?;
                let paths: Vec<String> = serde_json::from_str(&raw)
                    .map_err(|err| ClipboardXError::Validation(err.to_string()))?;
                Ok(ClipboardPayload::Files { paths })
            }
        }
    }
}

fn truncate_preview(s: &str) -> String {
    let normalized = s.trim();
    if normalized.chars().count() <= PREVIEW_TRUNCATE_CHARS {
        return normalized.to_owned();
    }
    let mut out: String = normalized.chars().take(PREVIEW_TRUNCATE_CHARS).collect();
    out.push('…');
    out
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
