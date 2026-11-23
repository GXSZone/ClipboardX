use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use clipboardx_model::{
    ArchiveError, ClipItem, ClipboardXError, Group, GroupId, VaultEntry, VaultEntryDraft,
};
use clipboardx_storage::HistoryStore;
use clipboardx_storage::db::NewClipRecord;
use rand::Rng;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::pinyin_index::build_pinyin_index;
use crate::vault_service::VaultService;

const ARCHIVE_AAD: &[u8] = b"clipboardx.cbx.archive.v1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;

/// Summary counts returned after exporting a `.cbx` migration package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CbxExportStats {
    pub history_exported: usize,
    pub groups_exported: usize,
    pub vault_exported: usize,
    pub blobs_exported: usize,
    pub encrypted: bool,
}

/// Summary counts returned after importing a `.cbx` migration package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CbxImportStats {
    pub history_imported: usize,
    pub history_skipped_duplicates: usize,
    pub groups_created: usize,
    pub vault_imported: usize,
    pub encrypted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CbxManifest {
    format_version: u8,
    created_at_ms: i64,
    encrypted: bool,
    history_count: usize,
    group_count: usize,
    vault_count: usize,
    salt_hex: Option<String>,
    nonce_hex: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CbxBundle {
    groups: Vec<Group>,
    history_items: Vec<ClipItem>,
    vault_entries: Vec<VaultEntry>,
    blobs_raw: BTreeMap<String, Vec<u8>>,
}

/// Offline `.cbx` migration packager supporting plaintext ZIP export or
/// `Argon2id + XChaCha20-Poly1305` authenticated full-bundle encryption.
pub struct CbxArchive;

impl CbxArchive {
    /// Exports history, groups, decrypted blobs (re-sealed cleanly), and optional unlocked vault entries.
    pub fn export_to_path(
        output_path: &Path,
        history: &HistoryStore,
        vault: Option<&VaultService>,
        passphrase: Option<&[u8]>,
    ) -> Result<CbxExportStats, ClipboardXError> {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| ArchiveError::Io(format!("create backup dir failed: {err}")))?;
        }

        let groups = history.list_groups()?;
        let history_items = history.export_all_items()?;

        let mut blobs_raw = BTreeMap::new();
        for item in &history_items {
            for hash in [item.blob_ref.as_deref(), item.summary.thumb_ref.as_deref()]
                .into_iter()
                .flatten()
            {
                if !blobs_raw.contains_key(hash)
                    && let Ok(bytes) = history.blobs().get_bytes(hash)
                {
                    blobs_raw.insert(hash.to_owned(), bytes);
                }
            }
        }

        let vault_entries = if let Some(v) = vault {
            v.list_entries("", 2000).unwrap_or_default()
        } else {
            Vec::new()
        };

        let bundle = CbxBundle {
            groups,
            history_items,
            vault_entries,
            blobs_raw,
        };

        let stats = CbxExportStats {
            history_exported: bundle.history_items.len(),
            groups_exported: bundle.groups.len(),
            vault_exported: bundle.vault_entries.len(),
            blobs_exported: bundle.blobs_raw.len(),
            encrypted: passphrase.is_some(),
        };

        let bundle_json = Zeroizing::new(serde_json::to_vec(&bundle).map_err(|err| {
            ArchiveError::InvalidFormat(format!("serialize bundle failed: {err}"))
        })?);

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let (manifest, encrypted_payload) = if let Some(pass) = passphrase {
            if pass.len() < 4 {
                return Err(ArchiveError::InvalidFormat(
                    "archive passphrase must be at least 4 bytes".to_owned(),
                )
                .into());
            }
            let mut salt = [0u8; SALT_LEN];
            rand::rng().fill_bytes(&mut salt);
            let mut nonce_bytes = [0u8; NONCE_LEN];
            rand::rng().fill_bytes(&mut nonce_bytes);

            let key = derive_archive_key(pass, &salt)?;
            let cipher = XChaCha20Poly1305::new((&*key).into());
            let ciphertext = cipher
                .encrypt(
                    &XNonce::from(nonce_bytes),
                    Payload {
                        msg: &bundle_json,
                        aad: ARCHIVE_AAD,
                    },
                )
                .map_err(|err| {
                    ArchiveError::InvalidFormat(format!("encrypt archive failed: {err}"))
                })?;

            let m = CbxManifest {
                format_version: 1,
                created_at_ms: now_ms,
                encrypted: true,
                history_count: stats.history_exported,
                group_count: stats.groups_exported,
                vault_count: stats.vault_exported,
                salt_hex: Some(hex::encode(salt)),
                nonce_hex: Some(hex::encode(nonce_bytes)),
            };
            (m, Some(ciphertext))
        } else {
            let m = CbxManifest {
                format_version: 1,
                created_at_ms: now_ms,
                encrypted: false,
                history_count: stats.history_exported,
                group_count: stats.groups_exported,
                vault_count: stats.vault_exported,
                salt_hex: None,
                nonce_hex: None,
            };
            (m, None)
        };

        let tmp_path = output_path.with_extension("cbx.tmp");
        {
            let file = File::create(&tmp_path)
                .map_err(|err| ArchiveError::Io(format!("create cbx archive failed: {err}")))?;
            let mut zip = ZipWriter::new(file);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

            let manifest_bytes = serde_json::to_vec_pretty(&manifest)
                .map_err(|err| ArchiveError::InvalidFormat(format!("serialize manifest: {err}")))?;
            zip.start_file("manifest.json", opts)
                .map_err(|err| ArchiveError::Io(format!("zip manifest start: {err}")))?;
            zip.write_all(&manifest_bytes)
                .map_err(|err| ArchiveError::Io(format!("zip manifest write: {err}")))?;

            if let Some(cipher_bytes) = encrypted_payload {
                zip.start_file("payload.enc", opts)
                    .map_err(|err| ArchiveError::Io(format!("zip payload start: {err}")))?;
                zip.write_all(&cipher_bytes)
                    .map_err(|err| ArchiveError::Io(format!("zip payload write: {err}")))?;
            } else {
                zip.start_file("payload.json", opts)
                    .map_err(|err| ArchiveError::Io(format!("zip payload start: {err}")))?;
                zip.write_all(&bundle_json)
                    .map_err(|err| ArchiveError::Io(format!("zip payload write: {err}")))?;
            }

            zip.finish()
                .map_err(|err| ArchiveError::Io(format!("zip finalize failed: {err}")))?;
        }

        fs::rename(&tmp_path, output_path)
            .map_err(|err| ArchiveError::Io(format!("commit cbx file failed: {err}")))?;

        Ok(stats)
    }

    /// Imports an archive package idempotently into `history` (and optionally an unlocked `vault`).
    pub fn import_from_path(
        input_path: &Path,
        history: &HistoryStore,
        vault: Option<&VaultService>,
        passphrase: Option<&[u8]>,
    ) -> Result<CbxImportStats, ClipboardXError> {
        let file = File::open(input_path)
            .map_err(|err| ArchiveError::Io(format!("open cbx archive failed: {err}")))?;
        let mut zip = ZipArchive::new(file).map_err(|err| {
            ArchiveError::InvalidFormat(format!("read zip container failed: {err}"))
        })?;

        let manifest: CbxManifest = {
            let mut mf = zip.by_name("manifest.json").map_err(|err| {
                ArchiveError::InvalidFormat(format!("missing manifest.json: {err}"))
            })?;
            let mut raw = Vec::new();
            mf.read_to_end(&mut raw)
                .map_err(|err| ArchiveError::Io(format!("read manifest failed: {err}")))?;
            serde_json::from_slice(&raw)
                .map_err(|err| ArchiveError::InvalidFormat(format!("corrupt manifest: {err}")))?
        };

        let bundle_bytes: Zeroizing<Vec<u8>> = if manifest.encrypted {
            let pass = passphrase.ok_or(ArchiveError::BadPassphrase)?;
            let salt_hex = manifest.salt_hex.as_deref().ok_or_else(|| {
                ArchiveError::InvalidFormat("missing salt in manifest".to_owned())
            })?;
            let nonce_hex = manifest.nonce_hex.as_deref().ok_or_else(|| {
                ArchiveError::InvalidFormat("missing nonce in manifest".to_owned())
            })?;

            let salt = hex::decode(salt_hex)
                .map_err(|err| ArchiveError::InvalidFormat(format!("decode salt: {err}")))?;
            let nonce_raw = hex::decode(nonce_hex)
                .map_err(|err| ArchiveError::InvalidFormat(format!("decode nonce: {err}")))?;
            if nonce_raw.len() != NONCE_LEN {
                return Err(
                    ArchiveError::InvalidFormat("invalid archive nonce length".to_owned()).into(),
                );
            }
            let mut nonce_arr = [0u8; NONCE_LEN];
            nonce_arr.copy_from_slice(&nonce_raw);

            let mut cipher_bytes = Vec::new();
            {
                let mut pf = zip.by_name("payload.enc").map_err(|err| {
                    ArchiveError::InvalidFormat(format!("missing payload.enc: {err}"))
                })?;
                pf.read_to_end(&mut cipher_bytes)
                    .map_err(|err| ArchiveError::Io(format!("read payload.enc: {err}")))?;
            }

            let key = derive_archive_key(pass, &salt)?;
            let cipher = XChaCha20Poly1305::new((&*key).into());
            Zeroizing::new(
                cipher
                    .decrypt(
                        &XNonce::from(nonce_arr),
                        Payload {
                            msg: &cipher_bytes,
                            aad: ARCHIVE_AAD,
                        },
                    )
                    .map_err(|_| ArchiveError::BadPassphrase)?,
            )
        } else {
            let mut pf = zip.by_name("payload.json").map_err(|err| {
                ArchiveError::InvalidFormat(format!("missing payload.json: {err}"))
            })?;
            let mut raw = Vec::new();
            pf.read_to_end(&mut raw)
                .map_err(|err| ArchiveError::Io(format!("read payload.json: {err}")))?;
            Zeroizing::new(raw)
        };

        let bundle: CbxBundle = serde_json::from_slice(&bundle_bytes)
            .map_err(|err| ArchiveError::InvalidFormat(format!("corrupt bundle payload: {err}")))?;

        // 1. Recreate groups and map old group IDs -> local DB group IDs.
        let existing_groups = history.list_groups()?;
        let mut groups_created = 0usize;
        let mut group_map: HashMap<i64, GroupId> = HashMap::new();
        for g in &bundle.groups {
            let already = existing_groups.iter().any(|eg| eg.name == g.name);
            let local_gid = history.create_group(&g.name, g.created_at_ms)?;
            if !already {
                groups_created += 1;
            }
            group_map.insert(g.id.0, local_gid);
        }

        // 2. Restore decrypted blob payloads into local CAS encrypted BlobStore.
        let mut blob_remap: HashMap<String, String> = HashMap::new();
        for (old_hash, plain_bytes) in &bundle.blobs_raw {
            let new_hash = history.blobs().put_bytes(plain_bytes)?;
            blob_remap.insert(old_hash.clone(), new_hash);
        }

        // 3. Merge history items idempotently by content_hash.
        let mut history_imported = 0usize;
        let mut history_skipped_duplicates = 0usize;
        for item in &bundle.history_items {
            let mapped_group = item
                .summary
                .group_id
                .and_then(|old_gid| group_map.get(&old_gid.0).copied());
            let mapped_blob = item
                .blob_ref
                .as_ref()
                .map(|old| blob_remap.get(old).cloned().unwrap_or_else(|| old.clone()));
            let mapped_thumb = item
                .summary
                .thumb_ref
                .as_ref()
                .map(|old| blob_remap.get(old).cloned().unwrap_or_else(|| old.clone()));

            let rec = NewClipRecord {
                content_hash: item.summary.content_hash.clone(),
                kind: item.summary.kind,
                subtype: item.summary.subtype,
                detected_lang: item.summary.detected_lang.clone(),
                preview_text: item.summary.preview_text.clone(),
                search_text: item.full_text.clone(),
                pinyin_index: build_pinyin_index(&item.full_text),
                payload_inline: item.payload_inline.clone(),
                blob_ref: mapped_blob,
                thumb_ref: mapped_thumb,
                meta_json: item.summary.meta_json.clone(),
                source_app_name: item.summary.source_app_name.clone(),
                source_app_id: None,
                byte_size: item.summary.byte_size,
                timestamp_ms: item.summary.updated_at_ms,
            };

            let inserted = history.import_item_idempotent(
                &rec,
                item.summary.is_pinned,
                mapped_group,
                &item.summary.alias,
                &item.summary.ocr_text,
                item.summary.created_at_ms,
            )?;
            if inserted {
                history_imported += 1;
            } else {
                history_skipped_duplicates += 1;
            }
        }

        // 4. Import vault entries when vault is attached and unlocked, skipping exact (title, user) duplicates.
        let mut vault_imported = 0usize;
        if let Some(v) = vault
            && !bundle.vault_entries.is_empty()
            && let Ok(existing_vault) = v.list_entries("", 5000)
        {
            for entry in &bundle.vault_entries {
                let exists = existing_vault
                    .iter()
                    .any(|e| e.title == entry.title && e.username == entry.username);
                if !exists {
                    let draft = VaultEntryDraft {
                        title: entry.title.clone(),
                        username: entry.username.clone(),
                        password: entry.password.clone(),
                        note: entry.note.clone(),
                        auto_submit: entry.auto_submit,
                    };
                    if v.create_entry(&draft).is_ok() {
                        vault_imported += 1;
                    }
                }
            }
        }

        Ok(CbxImportStats {
            history_imported,
            history_skipped_duplicates,
            groups_created,
            vault_imported,
            encrypted: manifest.encrypted,
        })
    }
}

fn derive_archive_key(
    passphrase: &[u8],
    salt: &[u8],
) -> Result<Zeroizing<[u8; KEY_LEN]>, ClipboardXError> {
    let params = Params::new(64 * 1024, 3, 1, Some(KEY_LEN))
        .map_err(|err| ArchiveError::InvalidFormat(format!("invalid argon2 params: {err}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(passphrase, salt, &mut *key)
        .map_err(|err| ArchiveError::InvalidFormat(format!("argon2id KDF failed: {err}")))?;
    Ok(key)
}
