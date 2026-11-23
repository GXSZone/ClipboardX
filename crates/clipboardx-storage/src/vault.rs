use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use clipboardx_model::{
    SecretStore, StorageError, VaultEntry, VaultEntryDraft, VaultEntryId, VaultUnlockPolicy,
};
use rand::Rng;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::migrations::migrate_vault_db;

const ENVELOPE_AAD: &[u8] = b"clipboardx.vault.envelope.v1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const DEK_LEN: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PassphraseEnvelope {
    version: u8,
    salt_hex: String,
    nonce_hex: String,
    wrapped_dek_hex: String,
}

struct VaultSession {
    conn: Connection,
    dek: Zeroizing<[u8; DEK_LEN]>,
    unlocked_at: Instant,
}

/// Encrypted isolated credential repository (`vault.db`) using envelope encryption
/// (`Argon2id` + `XChaCha20-Poly1305` wrapped 256-bit DEK) and full SQLite3MC ChaCha20 pages.
#[derive(Clone)]
pub struct VaultStore {
    db_path: PathBuf,
    envelope_path: PathBuf,
    session: Arc<Mutex<Option<VaultSession>>>,
}

impl VaultStore {
    /// Creates a store handle referencing `<vault_db_path>` and sidecar `<vault.key.json>`.
    pub fn new(db_path: PathBuf) -> Self {
        let envelope_path = db_path.with_extension("key.json");
        Self {
            db_path,
            envelope_path,
            session: Arc::new(Mutex::new(None)),
        }
    }

    pub fn exists(&self) -> bool {
        self.db_path.exists()
    }

    pub fn has_passphrase_envelope(&self) -> bool {
        self.envelope_path.exists()
    }

    /// Returns whether an active unlocked session exists and has not expired.
    /// Automatically locks and drops the DB connection when `PromptWithTimeout` expires.
    pub fn is_unlocked(&self, policy: VaultUnlockPolicy, timeout_minutes: u32) -> bool {
        let Ok(mut guard) = self.session.lock() else {
            return false;
        };
        let Some(sess) = guard.as_ref() else {
            return false;
        };

        if policy == VaultUnlockPolicy::PromptWithTimeout {
            let max_age = Duration::from_secs(u64::from(timeout_minutes.max(1)) * 60);
            if sess.unlocked_at.elapsed() >= max_age {
                *guard = None;
                return false;
            }
        }
        true
    }

    /// Drops the active DB connection and zeroes the in-memory DEK immediately.
    pub fn lock(&self) {
        if let Ok(mut guard) = self.session.lock() {
            *guard = None;
        }
    }

    /// Initializes or unlocks `vault.db` silently using the OS keyring trusted DEK cache.
    pub fn init_or_unlock_trusted(
        &self,
        secret_store: &dyn SecretStore,
    ) -> Result<(), StorageError> {
        let dek = match secret_store.load_vault_trusted_dek()? {
            Some(bytes) if bytes.len() == DEK_LEN => {
                let mut arr = Zeroizing::new([0u8; DEK_LEN]);
                arr.copy_from_slice(&bytes);
                arr
            }
            Some(_) => {
                return Err(StorageError::Keyring(
                    "stored trusted vault DEK has unexpected length".to_owned(),
                ));
            }
            None => {
                if self.db_path.exists() {
                    return Err(StorageError::VaultLocked);
                }
                let mut arr = Zeroizing::new([0u8; DEK_LEN]);
                rand::rng().fill_bytes(&mut *arr);
                secret_store.save_vault_trusted_dek(&*arr)?;
                arr
            }
        };

        self.open_with_dek(dek)
    }

    /// Creates a new encrypted `vault.db` sealed by an Argon2id passphrase envelope
    /// (and optionally caches a trusted DEK copy in `secret_store`).
    ///
    /// Refuses to overwrite or mismatch an already-existing `vault.db`; callers upgrading
    /// a trusted-keyring-only vault to passphrase protection must use
    /// [`Self::attach_passphrase_via_trusted`].
    pub fn init_with_passphrase(
        &self,
        passphrase: &[u8],
        trusted_secret_store: Option<&dyn SecretStore>,
    ) -> Result<(), StorageError> {
        validate_passphrase(passphrase)?;
        if self.envelope_path.exists() {
            return Err(StorageError::Database(
                "vault passphrase envelope already exists".to_owned(),
            ));
        }
        if self.db_path.exists() {
            return Err(StorageError::Crypto(
                "verify vault.db key failed: existing vault.db requires its original key"
                    .to_owned(),
            ));
        }

        let mut dek = Zeroizing::new([0u8; DEK_LEN]);
        rand::rng().fill_bytes(&mut *dek);

        let dek_copy = Zeroizing::new(*dek);
        self.open_with_dek(dek)?;
        if let Err(err) = write_passphrase_envelope(&self.envelope_path, passphrase, &dek_copy) {
            self.lock();
            return Err(err);
        }
        if let Some(store) = trusted_secret_store
            && let Err(err) = store.save_vault_trusted_dek(&*dek_copy)
        {
            tracing::warn!(error = %err, "cache trusted vault DEK after passphrase init failed");
        }

        Ok(())
    }

    /// Upgrades an existing trusted-keyring-only `vault.db` (without `vault.key.json`)
    /// by verifying the cached keyring DEK against `vault.db` first, then sealing that exact
    /// DEK inside a new passphrase envelope without ever clobbering the keyring or DB key.
    pub fn attach_passphrase_via_trusted(
        &self,
        passphrase: &[u8],
        secret_store: &dyn SecretStore,
        keep_trusted_cache: bool,
    ) -> Result<(), StorageError> {
        validate_passphrase(passphrase)?;
        if self.envelope_path.exists() {
            return Err(StorageError::Database(
                "vault passphrase envelope already exists".to_owned(),
            ));
        }

        let raw_dek = secret_store.load_vault_trusted_dek()?.ok_or_else(|| {
            StorageError::Crypto(
                "verify vault.db key failed: trusted keyring DEK missing for existing vault.db"
                    .to_owned(),
            )
        })?;
        if raw_dek.len() != DEK_LEN {
            return Err(StorageError::Keyring(
                "stored trusted vault DEK has unexpected length".to_owned(),
            ));
        }
        let mut dek = Zeroizing::new([0u8; DEK_LEN]);
        dek.copy_from_slice(&raw_dek);
        let dek_copy = Zeroizing::new(*dek);

        // Verify the keyring DEK actually decrypts the existing database before writing the envelope.
        self.open_with_dek(dek)?;
        if let Err(err) = write_passphrase_envelope(&self.envelope_path, passphrase, &dek_copy) {
            self.lock();
            return Err(err);
        }
        if !keep_trusted_cache {
            let _ = secret_store.clear_vault_trusted_dek();
        }
        Ok(())
    }

    /// Permanently removes a mismatched or corrupted `vault.db`, its WAL/SHM sidecars,
    /// `vault.key.json`, and cached trusted DEK so the user can cleanly re-initialize.
    pub fn reset_vault(&self, secret_store: &dyn SecretStore) -> Result<(), StorageError> {
        self.lock();
        let wal_path = PathBuf::from(format!("{}-wal", self.db_path.display()));
        let shm_path = PathBuf::from(format!("{}-shm", self.db_path.display()));
        for path in [&self.db_path, &wal_path, &shm_path, &self.envelope_path] {
            if path.exists() {
                fs::remove_file(path).map_err(|err| {
                    StorageError::BlobIo(format!("remove vault artifact failed: {err}"))
                })?;
            }
        }
        secret_store.clear_vault_trusted_dek()?;
        Ok(())
    }

    /// Unlocks `vault.db` using the user's master passphrase envelope.
    pub fn unlock_with_passphrase(&self, passphrase: &[u8]) -> Result<(), StorageError> {
        let dek = unwrap_passphrase_envelope(&self.envelope_path, passphrase)?;
        self.open_with_dek(dek)
    }

    /// Attaches a passphrase envelope to a vault previously initialized only via trusted keyring,
    /// using the currently unlocked in-memory DEK.
    pub fn attach_passphrase_while_unlocked(
        &self,
        new_passphrase: &[u8],
    ) -> Result<(), StorageError> {
        validate_passphrase(new_passphrase)?;
        let dek = {
            let guard = self
                .session
                .lock()
                .map_err(|_| StorageError::Database("vault mutex poisoned".to_owned()))?;
            let sess = guard.as_ref().ok_or(StorageError::VaultLocked)?;
            Zeroizing::new(*sess.dek)
        };
        write_passphrase_envelope(&self.envelope_path, new_passphrase, &dek)
    }

    /// Rotates the envelope passphrase without re-encrypting the underlying database pages.
    pub fn change_passphrase(
        &self,
        current_passphrase: &[u8],
        new_passphrase: &[u8],
    ) -> Result<(), StorageError> {
        validate_passphrase(new_passphrase)?;
        let dek = unwrap_passphrase_envelope(&self.envelope_path, current_passphrase)?;
        write_passphrase_envelope(&self.envelope_path, new_passphrase, &dek)
    }

    /// Caches the currently unlocked DEK into OS keyring for trusted passwordless unlocks.
    pub fn enable_trusted_cache(&self, secret_store: &dyn SecretStore) -> Result<(), StorageError> {
        let guard = self
            .session
            .lock()
            .map_err(|_| StorageError::Database("vault mutex poisoned".to_owned()))?;
        let sess = guard.as_ref().ok_or(StorageError::VaultLocked)?;
        secret_store.save_vault_trusted_dek(&*sess.dek)
    }

    /// Inserts a new credential entry into the unlocked vault.
    pub fn create_entry(
        &self,
        draft: &VaultEntryDraft,
        pinyin_index: &str,
        now_ms: i64,
    ) -> Result<VaultEntryId, StorageError> {
        let title = draft.title.trim();
        if title.is_empty() || draft.username.trim().is_empty() {
            return Err(StorageError::Database(
                "vault entry requires non-empty title and username".to_owned(),
            ));
        }

        let mut guard = self
            .session
            .lock()
            .map_err(|_| StorageError::Database("vault mutex poisoned".to_owned()))?;
        let sess = guard.as_mut().ok_or(StorageError::VaultLocked)?;
        sess.unlocked_at = Instant::now();

        sess.conn
            .execute(
                "INSERT INTO vault_entries (title, username, password, note, auto_submit, pinyin_index, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    title,
                    draft.username.trim(),
                    draft.password,
                    draft.note.trim(),
                    i64::from(draft.auto_submit),
                    pinyin_index,
                    now_ms,
                ],
            )
            .map_err(|err| StorageError::Database(format!("insert vault entry failed: {err}")))?;

        Ok(VaultEntryId(sess.conn.last_insert_rowid()))
    }

    /// Updates an existing credential entry.
    pub fn update_entry(
        &self,
        id: VaultEntryId,
        draft: &VaultEntryDraft,
        pinyin_index: &str,
        now_ms: i64,
    ) -> Result<(), StorageError> {
        let title = draft.title.trim();
        if title.is_empty() || draft.username.trim().is_empty() {
            return Err(StorageError::Database(
                "vault entry requires non-empty title and username".to_owned(),
            ));
        }

        let mut guard = self
            .session
            .lock()
            .map_err(|_| StorageError::Database("vault mutex poisoned".to_owned()))?;
        let sess = guard.as_mut().ok_or(StorageError::VaultLocked)?;
        sess.unlocked_at = Instant::now();

        let affected = sess
            .conn
            .execute(
                "UPDATE vault_entries
                 SET title = ?1, username = ?2, password = ?3, note = ?4,
                     auto_submit = ?5, pinyin_index = ?6, updated_at = ?7
                 WHERE id = ?8",
                params![
                    title,
                    draft.username.trim(),
                    draft.password,
                    draft.note.trim(),
                    i64::from(draft.auto_submit),
                    pinyin_index,
                    now_ms,
                    id.0,
                ],
            )
            .map_err(|err| StorageError::Database(format!("update vault entry failed: {err}")))?;

        if affected == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Toggles per-entry `auto_submit` flag directly.
    pub fn toggle_auto_submit(&self, id: VaultEntryId, now_ms: i64) -> Result<bool, StorageError> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| StorageError::Database("vault mutex poisoned".to_owned()))?;
        let sess = guard.as_mut().ok_or(StorageError::VaultLocked)?;
        sess.unlocked_at = Instant::now();

        sess.conn
            .execute(
                "UPDATE vault_entries
                 SET auto_submit = CASE WHEN auto_submit = 0 THEN 1 ELSE 0 END,
                     updated_at = ?1
                 WHERE id = ?2",
                params![now_ms, id.0],
            )
            .map_err(|err| StorageError::Database(format!("toggle auto_submit failed: {err}")))?;

        let val: i64 = sess
            .conn
            .query_row(
                "SELECT auto_submit FROM vault_entries WHERE id = ?1",
                params![id.0],
                |r| r.get(0),
            )
            .map_err(|err| StorageError::Database(format!("read auto_submit failed: {err}")))?;
        Ok(val != 0)
    }

    /// Deletes a credential entry permanently.
    pub fn delete_entry(&self, id: VaultEntryId) -> Result<(), StorageError> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| StorageError::Database("vault mutex poisoned".to_owned()))?;
        let sess = guard.as_mut().ok_or(StorageError::VaultLocked)?;
        sess.unlocked_at = Instant::now();

        sess.conn
            .execute("DELETE FROM vault_entries WHERE id = ?1", params![id.0])
            .map_err(|err| StorageError::Database(format!("delete vault entry failed: {err}")))?;
        Ok(())
    }

    /// Lists or searches entries inside the unlocked vault by title, username, note, or pinyin.
    pub fn list_entries(&self, query: &str, limit: usize) -> Result<Vec<VaultEntry>, StorageError> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| StorageError::Database("vault mutex poisoned".to_owned()))?;
        let sess = guard.as_mut().ok_or(StorageError::VaultLocked)?;
        sess.unlocked_at = Instant::now();

        let trimmed = query.trim();
        let mut out = Vec::new();
        if trimmed.is_empty() {
            let mut stmt = sess
                .conn
                .prepare(
                    "SELECT id, title, username, password, note, auto_submit, pinyin_index, updated_at
                     FROM vault_entries
                     ORDER BY updated_at DESC
                     LIMIT ?1",
                )
                .map_err(|err| StorageError::Database(format!("list_entries prepare: {err}")))?;
            let rows = stmt
                .query_map(params![limit as i64], map_vault_row)
                .map_err(|err| StorageError::Database(format!("list_entries query: {err}")))?;
            for r in rows {
                out.push(r.map_err(|err| StorageError::Database(err.to_string()))?);
            }
        } else {
            let pattern = format!("%{trimmed}%");
            let mut stmt = sess
                .conn
                .prepare(
                    "SELECT id, title, username, password, note, auto_submit, pinyin_index, updated_at
                     FROM vault_entries
                     WHERE title LIKE ?1 OR username LIKE ?1 OR note LIKE ?1 OR pinyin_index LIKE ?1
                     ORDER BY updated_at DESC
                     LIMIT ?2",
                )
                .map_err(|err| StorageError::Database(format!("search vault prepare: {err}")))?;
            let rows = stmt
                .query_map(params![pattern, limit as i64], map_vault_row)
                .map_err(|err| StorageError::Database(format!("search vault query: {err}")))?;
            for r in rows {
                out.push(r.map_err(|err| StorageError::Database(err.to_string()))?);
            }
        }
        Ok(out)
    }

    /// Fetches a single unlocked vault entry by ID.
    pub fn get_entry(&self, id: VaultEntryId) -> Result<VaultEntry, StorageError> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| StorageError::Database("vault mutex poisoned".to_owned()))?;
        let sess = guard.as_mut().ok_or(StorageError::VaultLocked)?;
        sess.unlocked_at = Instant::now();

        let entry = sess
            .conn
            .query_row(
                "SELECT id, title, username, password, note, auto_submit, pinyin_index, updated_at
                 FROM vault_entries
                 WHERE id = ?1",
                params![id.0],
                map_vault_row,
            )
            .optional()
            .map_err(|err| StorageError::Database(format!("get_entry failed: {err}")))?;

        entry.ok_or_else(|| StorageError::NotFound(id.to_string()))
    }

    fn open_with_dek(&self, dek: Zeroizing<[u8; DEK_LEN]>) -> Result<(), StorageError> {
        if let Some(parent) = self.db_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| StorageError::Database(format!("create vault dir failed: {err}")))?;
        }

        let conn = Connection::open(&self.db_path)
            .map_err(|err| StorageError::Database(format!("open vault.db failed: {err}")))?;
        let dek_hex = Zeroizing::new(hex::encode(*dek));
        let pragma_sql = Zeroizing::new(format!(
            "PRAGMA cipher = 'chacha20'; PRAGMA key = \"x'{}'\"; PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;",
            *dek_hex
        ));
        conn.execute_batch(&pragma_sql).map_err(|err| {
            StorageError::Crypto(format!("unlock encrypted vault.db failed: {err}"))
        })?;

        let _: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
            .map_err(|err| StorageError::Crypto(format!("verify vault.db key failed: {err}")))?;

        migrate_vault_db(&conn)?;

        let mut guard = self
            .session
            .lock()
            .map_err(|_| StorageError::Database("vault mutex poisoned".to_owned()))?;
        *guard = Some(VaultSession {
            conn,
            dek,
            unlocked_at: Instant::now(),
        });
        Ok(())
    }
}

fn validate_passphrase(passphrase: &[u8]) -> Result<(), StorageError> {
    if passphrase.len() < 4 {
        return Err(StorageError::Crypto(
            "vault passphrase must be at least 4 bytes".to_owned(),
        ));
    }
    Ok(())
}

fn derive_kek(passphrase: &[u8], salt: &[u8]) -> Result<Zeroizing<[u8; DEK_LEN]>, StorageError> {
    let params = Params::new(64 * 1024, 3, 1, Some(DEK_LEN))
        .map_err(|err| StorageError::Crypto(format!("invalid argon2 params: {err}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut kek = Zeroizing::new([0u8; DEK_LEN]);
    argon2
        .hash_password_into(passphrase, salt, &mut *kek)
        .map_err(|err| StorageError::Crypto(format!("argon2id KDF failed: {err}")))?;
    Ok(kek)
}

fn write_passphrase_envelope(
    path: &Path,
    passphrase: &[u8],
    dek: &[u8; DEK_LEN],
) -> Result<(), StorageError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| StorageError::BlobIo(format!("create vault envelope dir: {err}")))?;
    }

    let mut salt = [0u8; SALT_LEN];
    rand::rng().fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut nonce_bytes);

    let kek = derive_kek(passphrase, &salt)?;
    let cipher = XChaCha20Poly1305::new((&*kek).into());
    let nonce = XNonce::from(nonce_bytes);
    let wrapped = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: dek,
                aad: ENVELOPE_AAD,
            },
        )
        .map_err(|err| StorageError::Crypto(format!("encrypt vault DEK failed: {err}")))?;

    let envelope = PassphraseEnvelope {
        version: 1,
        salt_hex: hex::encode(salt),
        nonce_hex: hex::encode(nonce_bytes),
        wrapped_dek_hex: hex::encode(wrapped),
    };
    let json = serde_json::to_vec_pretty(&envelope)
        .map_err(|err| StorageError::BlobIo(format!("serialize envelope failed: {err}")))?;

    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, json)
        .map_err(|err| StorageError::BlobIo(format!("write temp envelope failed: {err}")))?;
    fs::rename(&tmp_path, path)
        .map_err(|err| StorageError::BlobIo(format!("commit envelope failed: {err}")))?;
    Ok(())
}

fn unwrap_passphrase_envelope(
    path: &Path,
    passphrase: &[u8],
) -> Result<Zeroizing<[u8; DEK_LEN]>, StorageError> {
    if !path.exists() {
        return Err(StorageError::NotFound(
            "vault passphrase envelope missing".to_owned(),
        ));
    }

    let raw = fs::read(path)
        .map_err(|err| StorageError::BlobIo(format!("read vault envelope failed: {err}")))?;
    let envelope: PassphraseEnvelope = serde_json::from_slice(&raw)
        .map_err(|err| StorageError::Crypto(format!("corrupt vault envelope: {err}")))?;

    let salt = hex::decode(&envelope.salt_hex)
        .map_err(|err| StorageError::Crypto(format!("decode salt failed: {err}")))?;
    let nonce_raw = hex::decode(&envelope.nonce_hex)
        .map_err(|err| StorageError::Crypto(format!("decode nonce failed: {err}")))?;
    let wrapped = hex::decode(&envelope.wrapped_dek_hex)
        .map_err(|err| StorageError::Crypto(format!("decode wrapped DEK failed: {err}")))?;

    if nonce_raw.len() != NONCE_LEN {
        return Err(StorageError::Crypto(
            "invalid vault envelope nonce length".to_owned(),
        ));
    }
    let mut nonce_arr = [0u8; NONCE_LEN];
    nonce_arr.copy_from_slice(&nonce_raw);

    let kek = derive_kek(passphrase, &salt)?;
    let cipher = XChaCha20Poly1305::new((&*kek).into());
    let plain = Zeroizing::new(
        cipher
            .decrypt(
                &XNonce::from(nonce_arr),
                Payload {
                    msg: &wrapped,
                    aad: ENVELOPE_AAD,
                },
            )
            .map_err(|_| StorageError::Crypto("invalid vault passphrase".to_owned()))?,
    );

    if plain.len() != DEK_LEN {
        return Err(StorageError::Crypto(
            "decrypted vault DEK has invalid length".to_owned(),
        ));
    }

    let mut dek = Zeroizing::new([0u8; DEK_LEN]);
    dek.copy_from_slice(&plain);
    Ok(dek)
}

fn map_vault_row(row: &Row<'_>) -> rusqlite::Result<VaultEntry> {
    let auto_submit: i64 = row.get(5)?;
    Ok(VaultEntry {
        id: VaultEntryId(row.get(0)?),
        title: row.get(1)?,
        username: row.get(2)?,
        password: row.get(3)?,
        note: row.get(4)?,
        auto_submit: auto_submit != 0,
        pinyin_index: row.get(6)?,
        updated_at_ms: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MemorySecretStore {
        trusted_dek: Mutex<Option<Vec<u8>>>,
    }

    impl SecretStore for MemorySecretStore {
        fn get_or_create_master_key(&self) -> Result<Zeroizing<Vec<u8>>, StorageError> {
            Ok(Zeroizing::new(vec![0xAB; 32]))
        }

        fn save_vault_trusted_dek(&self, dek: &[u8]) -> Result<(), StorageError> {
            *self.trusted_dek.lock().unwrap() = Some(dek.to_vec());
            Ok(())
        }

        fn load_vault_trusted_dek(&self) -> Result<Option<Zeroizing<Vec<u8>>>, StorageError> {
            Ok(self.trusted_dek.lock().unwrap().clone().map(Zeroizing::new))
        }

        fn clear_vault_trusted_dek(&self) -> Result<(), StorageError> {
            *self.trusted_dek.lock().unwrap() = None;
            Ok(())
        }
    }

    fn temp_vault_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clipboardx_vault_{label}_{}",
            rand::random::<u64>()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn trusted_vault_upgrades_to_passphrase_without_clobbering_existing_entries() {
        let dir = temp_vault_dir("upgrade");
        let store = VaultStore::new(dir.join("vault.db"));
        let secrets = MemorySecretStore::default();

        store.init_or_unlock_trusted(&secrets).unwrap();
        let draft = VaultEntryDraft {
            title: "GitHub".to_owned(),
            username: "octocat".to_owned(),
            password: "secret-password".to_owned(),
            note: "prod".to_owned(),
            auto_submit: true,
        };
        store.create_entry(&draft, "github", 1000).unwrap();
        store.lock();

        // Upgrading after lock must reuse the trusted DEK instead of generating a conflicting one.
        store
            .attach_passphrase_via_trusted(b"1234", &secrets, true)
            .unwrap();
        assert!(store.has_passphrase_envelope());
        assert_eq!(store.list_entries("", 10).unwrap().len(), 1);

        // Locking and unlocking via the newly attached passphrase must decrypt the same DB.
        store.lock();
        store.unlock_with_passphrase(b"1234").unwrap();
        let entries = store.list_entries("", 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].username, "octocat");

        // Wrong passphrase must fail cleanly.
        store.lock();
        assert!(store.unlock_with_passphrase(b"9999").is_err());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn init_with_passphrase_refuses_to_clobber_existing_db_and_reset_recovers() {
        let dir = temp_vault_dir("no_clobber");
        let store = VaultStore::new(dir.join("vault.db"));
        let secrets = MemorySecretStore::default();

        store.init_or_unlock_trusted(&secrets).unwrap();
        store.lock();
        secrets.clear_vault_trusted_dek().unwrap();

        // Existing vault.db without keyring DEK must fail without leaving a mismatched envelope.
        assert!(store.init_with_passphrase(b"1234", Some(&secrets)).is_err());
        assert!(!store.has_passphrase_envelope());
        assert!(secrets.load_vault_trusted_dek().unwrap().is_none());

        // Explicit reset clears the orphaned DB and allows a clean passphrase initialization.
        store.reset_vault(&secrets).unwrap();
        assert!(!store.exists());
        store.init_with_passphrase(b"1234", Some(&secrets)).unwrap();
        assert!(store.has_passphrase_envelope());
        assert!(store.list_entries("", 10).unwrap().is_empty());

        let _ = fs::remove_dir_all(dir);
    }
}
