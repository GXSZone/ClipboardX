use std::fs;
use std::sync::Arc;

use clipboardx_core::{CbxArchive, ClipboardService, VaultService};
use clipboardx_model::{
    AppConfig, CapturedClipboardEvent, ClipboardPayload, FilterTab, SecretStore, StorageError,
    VaultEntryDraft,
};
use clipboardx_storage::{BlobStore, HistoryStore, VaultStore};
use zeroize::Zeroizing;

struct StubSecretStore;

impl SecretStore for StubSecretStore {
    fn get_or_create_master_key(&self) -> Result<Zeroizing<Vec<u8>>, StorageError> {
        Ok(Zeroizing::new(vec![0x11; 32]))
    }

    fn save_vault_trusted_dek(&self, _: &[u8]) -> Result<(), StorageError> {
        Ok(())
    }

    fn load_vault_trusted_dek(&self) -> Result<Option<Zeroizing<Vec<u8>>>, StorageError> {
        Ok(None)
    }

    fn clear_vault_trusted_dek(&self) -> Result<(), StorageError> {
        Ok(())
    }
}

fn temp_dir(name: &str) -> Result<std::path::PathBuf, std::io::Error> {
    let dir = std::env::temp_dir().join(format!(
        "clipboardx_archive_test_{}_{name}",
        std::process::id()
    ));
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[test]
fn encrypted_cbx_archive_roundtrip_preserves_history_groups_and_vault_without_plaintext_leak() {
    let dir = match temp_dir("roundtrip") {
        Ok(d) => d,
        Err(err) => panic!("temp dir failed: {err}"),
    };
    let src_dir = dir.join("src");
    let dst_dir = dir.join("dst");
    let cbx_path = dir.join("backup.cbx");

    let key_hex = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
    let src_blobs = match BlobStore::new(src_dir.join("blobs"), Zeroizing::new([0x22; 32])) {
        Ok(b) => b,
        Err(err) => panic!("src blobs: {err}"),
    };
    let src_history = match HistoryStore::open(&src_dir.join("history.db"), key_hex, src_blobs) {
        Ok(h) => h,
        Err(err) => panic!("src history: {err}"),
    };
    let src_svc = ClipboardService::new(src_history.clone());

    let gid = match src_svc.create_group("DesignTokens") {
        Ok(id) => id,
        Err(err) => panic!("create_group: {err}"),
    };
    let cfg = AppConfig::default();
    let clip_secret = "SecretArchiveContent#9988";
    let clip_id = match src_svc.ingest_event(
        CapturedClipboardEvent {
            payload: ClipboardPayload::Text {
                plain: clip_secret.to_owned(),
            },
            source_app_name: Some("Editor".to_owned()),
            source_app_id: None,
            captured_at_ms: 123_456,
        },
        &cfg,
    ) {
        Ok(Some(id)) => id,
        other => panic!("ingest failed: {other:?}"),
    };
    assert!(src_svc.toggle_pin(clip_id).is_ok());
    assert!(src_svc.update_alias(clip_id, "Brand Color").is_ok());
    assert!(src_svc.assign_group(clip_id, Some(gid)).is_ok());

    let src_vault = VaultService::new(
        VaultStore::new(src_dir.join("vault.db")),
        Arc::new(StubSecretStore),
    );
    assert!(
        src_vault
            .init_with_passphrase(b"vault-pass-1", false)
            .is_ok()
    );
    let vault_secret = "VaultPassword!Archive77";
    assert!(
        src_vault
            .create_entry(&VaultEntryDraft {
                title: "GitLab Corp".to_owned(),
                username: "guo@corp.local".to_owned(),
                password: vault_secret.to_owned(),
                note: "Internal git".to_owned(),
                auto_submit: true,
            })
            .is_ok()
    );

    let export_stats = match CbxArchive::export_to_path(
        &cbx_path,
        &src_history,
        Some(&src_vault),
        Some(b"export-pass-2026"),
    ) {
        Ok(s) => s,
        Err(err) => panic!("export failed: {err}"),
    };
    assert!(export_stats.encrypted);
    assert_eq!(export_stats.history_exported, 1);
    assert_eq!(export_stats.groups_exported, 1);
    assert_eq!(export_stats.vault_exported, 1);

    // Raw .cbx bytes must not expose history or vault plaintext.
    let raw_zip = match fs::read(&cbx_path) {
        Ok(b) => b,
        Err(err) => panic!("read cbx: {err}"),
    };
    assert!(
        !raw_zip
            .windows(clip_secret.len())
            .any(|w| w == clip_secret.as_bytes())
    );
    assert!(
        !raw_zip
            .windows(vault_secret.len())
            .any(|w| w == vault_secret.as_bytes())
    );

    // Target store setup.
    let dst_blobs = match BlobStore::new(dst_dir.join("blobs"), Zeroizing::new([0x33; 32])) {
        Ok(b) => b,
        Err(err) => panic!("dst blobs: {err}"),
    };
    let dst_history = match HistoryStore::open(&dst_dir.join("history.db"), key_hex, dst_blobs) {
        Ok(h) => h,
        Err(err) => panic!("dst history: {err}"),
    };
    let dst_vault = VaultService::new(
        VaultStore::new(dst_dir.join("vault.db")),
        Arc::new(StubSecretStore),
    );
    assert!(
        dst_vault
            .init_with_passphrase(b"vault-pass-2", false)
            .is_ok()
    );

    // Wrong passphrase must fail.
    assert!(
        CbxArchive::import_from_path(&cbx_path, &dst_history, Some(&dst_vault), Some(b"wrong"))
            .is_err()
    );

    // Correct passphrase imports everything cleanly.
    let import_stats = match CbxArchive::import_from_path(
        &cbx_path,
        &dst_history,
        Some(&dst_vault),
        Some(b"export-pass-2026"),
    ) {
        Ok(s) => s,
        Err(err) => panic!("import failed: {err}"),
    };
    assert_eq!(import_stats.history_imported, 1);
    assert_eq!(import_stats.groups_created, 1);
    assert_eq!(import_stats.vault_imported, 1);

    let dst_svc = ClipboardService::new(dst_history.clone());
    let imported_items = match dst_svc.query_summaries(FilterTab::Pinned, None, "", 10) {
        Ok(v) => v,
        Err(err) => panic!("query pinned: {err}"),
    };
    assert_eq!(imported_items.len(), 1);
    assert_eq!(imported_items[0].alias, "Brand Color");
    assert!(imported_items[0].group_id.is_some());

    let imported_vault = match dst_vault.list_entries("GitLab", 10) {
        Ok(v) => v,
        Err(err) => panic!("list vault: {err}"),
    };
    assert_eq!(imported_vault.len(), 1);
    assert_eq!(imported_vault[0].password, vault_secret);

    // Re-importing the same package skips duplicates cleanly.
    let second_import = match CbxArchive::import_from_path(
        &cbx_path,
        &dst_history,
        Some(&dst_vault),
        Some(b"export-pass-2026"),
    ) {
        Ok(s) => s,
        Err(err) => panic!("second import: {err}"),
    };
    assert_eq!(second_import.history_imported, 0);
    assert_eq!(second_import.history_skipped_duplicates, 1);
    assert_eq!(second_import.vault_imported, 0);

    let _ = fs::remove_dir_all(&dir);
}
