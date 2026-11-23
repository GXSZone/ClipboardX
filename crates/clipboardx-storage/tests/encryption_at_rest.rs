use std::fs;
use std::path::PathBuf;

use clipboardx_model::{ClipKind, RetentionPolicy, SmartSubtype};
use clipboardx_storage::crypto::{BlobStore, CryptoKeys, sha256_hex};
use clipboardx_storage::db::{HistoryStore, NewClipRecord};
use rusqlite::Connection;

fn temp_dir(label: &str) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("clipboardx_it_{label}_{}", rand::random::<u64>()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[test]
fn encryption_at_rest_hides_plaintext_and_fts_index() {
    let dir = temp_dir("enc_rest").unwrap();
    let db_path = dir.join("history.db");
    let blob_dir = dir.join("blobs");

    let master = [0x99u8; 32];
    let keys = CryptoKeys::derive_from_master(&master).unwrap();
    let blobs = BlobStore::new(blob_dir, keys.blob_key.clone()).unwrap();
    let store = HistoryStore::open(&db_path, &keys.db_key_hex, blobs).unwrap();

    let sensitive_marker = "TOP_SECRET_CLIPBOARD_TOKEN_XYZ_987654";
    store
        .upsert_item(&NewClipRecord {
            content_hash: sha256_hex(sensitive_marker.as_bytes()),
            kind: ClipKind::Text,
            subtype: SmartSubtype::Plain,
            detected_lang: None,
            preview_text: sensitive_marker.to_owned(),
            search_text: sensitive_marker.to_owned(),
            pinyin_index: String::new(),
            payload_inline: Some(sensitive_marker.to_owned()),
            blob_ref: None,
            thumb_ref: None,
            meta_json: "{}".to_owned(),
            source_app_name: Some("IntegrationTest".to_owned()),
            source_app_id: Some("test.app".to_owned()),
            byte_size: sensitive_marker.len() as u64,
            timestamp_ms: 1_763_800_000_000,
        })
        .unwrap();

    drop(store);

    // 1. Raw database bytes on disk must NEVER contain the plaintext or FTS tokens.
    let raw_db = fs::read(&db_path).unwrap();
    assert!(
        !raw_db
            .windows(sensitive_marker.len())
            .any(|w| w == sensitive_marker.as_bytes())
    );

    // 2. Opening without key must fail.
    let unkeyed = Connection::open(&db_path).unwrap();
    assert!(
        unkeyed
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r
                .get::<_, i64>(0))
            .is_err()
    );

    // 3. Opening with wrong key must fail.
    let wrong_keys = CryptoKeys::derive_from_master(&[0x11u8; 32]).unwrap();
    let wrong_blobs = BlobStore::new(dir.join("blobs"), wrong_keys.blob_key).unwrap();
    assert!(HistoryStore::open(&db_path, &wrong_keys.db_key_hex, wrong_blobs).is_err());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn deduplication_and_retention_exempt_pinned_items() {
    let dir = temp_dir("retention").unwrap();
    let db_path = dir.join("history.db");
    let keys = CryptoKeys::derive_from_master(&[0x55u8; 32]).unwrap();
    let blobs = BlobStore::new(dir.join("blobs"), keys.blob_key.clone()).unwrap();
    let store = HistoryStore::open(&db_path, &keys.db_key_hex, blobs).unwrap();

    let id1 = store
        .upsert_item(&NewClipRecord {
            content_hash: sha256_hex(b"pinned-item"),
            kind: ClipKind::Text,
            subtype: SmartSubtype::Plain,
            detected_lang: None,
            preview_text: "pinned-item".to_owned(),
            search_text: "pinned-item".to_owned(),
            pinyin_index: String::new(),
            payload_inline: Some("pinned-item".to_owned()),
            blob_ref: None,
            thumb_ref: None,
            meta_json: "{}".to_owned(),
            source_app_name: None,
            source_app_id: None,
            byte_size: 11,
            timestamp_ms: 1000,
        })
        .unwrap();

    assert!(store.toggle_pin(id1).unwrap());
    store.update_alias(id1, "keep-forever").unwrap();

    // Duplicate insertion bumps timestamp while preserving pin and alias.
    let id1_dup = store
        .upsert_item(&NewClipRecord {
            content_hash: sha256_hex(b"pinned-item"),
            kind: ClipKind::Text,
            subtype: SmartSubtype::Plain,
            detected_lang: None,
            preview_text: "pinned-item".to_owned(),
            search_text: "pinned-item".to_owned(),
            pinyin_index: String::new(),
            payload_inline: Some("pinned-item".to_owned()),
            blob_ref: None,
            thumb_ref: None,
            meta_json: "{}".to_owned(),
            source_app_name: Some("Terminal".to_owned()),
            source_app_id: None,
            byte_size: 11,
            timestamp_ms: 2000,
        })
        .unwrap();
    assert_eq!(id1, id1_dup);

    // Insert an old unpinned item that should be pruned.
    store
        .upsert_item(&NewClipRecord {
            content_hash: sha256_hex(b"old-unpinned"),
            kind: ClipKind::Text,
            subtype: SmartSubtype::Plain,
            detected_lang: None,
            preview_text: "old-unpinned".to_owned(),
            search_text: "old-unpinned".to_owned(),
            pinyin_index: String::new(),
            payload_inline: Some("old-unpinned".to_owned()),
            blob_ref: None,
            thumb_ref: None,
            meta_json: "{}".to_owned(),
            source_app_name: None,
            source_app_id: None,
            byte_size: 12,
            timestamp_ms: 500,
        })
        .unwrap();

    let policy = RetentionPolicy {
        max_days: 30,
        max_items: 2000,
        max_item_bytes: 1024 * 1024,
    };
    let pruned = store
        .prune_retention(&policy, 500 + 35 * 86_400_000)
        .unwrap();
    assert_eq!(pruned, 1);

    let loaded = store.get_item(id1).unwrap();
    assert!(loaded.summary.is_pinned);
    assert_eq!(loaded.summary.alias, "keep-forever");
    assert_eq!(loaded.summary.use_count, 2);

    let _ = fs::remove_dir_all(dir);
}
