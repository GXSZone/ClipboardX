use std::fs;
use std::sync::Mutex;

use clipboardx_model::{SecretStore, StorageError, VaultEntryDraft, VaultUnlockPolicy};
use clipboardx_storage::VaultStore;
use rand::Rng;
use zeroize::Zeroizing;

struct MemorySecretStore {
    trusted_dek: Mutex<Option<Vec<u8>>>,
}

impl MemorySecretStore {
    fn new() -> Self {
        Self {
            trusted_dek: Mutex::new(None),
        }
    }
}

impl SecretStore for MemorySecretStore {
    fn get_or_create_master_key(&self) -> Result<Zeroizing<Vec<u8>>, StorageError> {
        let mut buf = Zeroizing::new(vec![0u8; 32]);
        rand::rng().fill_bytes(&mut buf);
        Ok(buf)
    }

    fn save_vault_trusted_dek(&self, dek: &[u8]) -> Result<(), StorageError> {
        let mut guard = self
            .trusted_dek
            .lock()
            .map_err(|_| StorageError::Keyring("lock failed".to_owned()))?;
        *guard = Some(dek.to_vec());
        Ok(())
    }

    fn load_vault_trusted_dek(&self) -> Result<Option<Zeroizing<Vec<u8>>>, StorageError> {
        let guard = self
            .trusted_dek
            .lock()
            .map_err(|_| StorageError::Keyring("lock failed".to_owned()))?;
        Ok(guard.as_ref().map(|v| Zeroizing::new(v.clone())))
    }

    fn clear_vault_trusted_dek(&self) -> Result<(), StorageError> {
        let mut guard = self
            .trusted_dek
            .lock()
            .map_err(|_| StorageError::Keyring("lock failed".to_owned()))?;
        *guard = None;
        Ok(())
    }
}

fn temp_vault_dir(name: &str) -> Result<std::path::PathBuf, std::io::Error> {
    let dir = std::env::temp_dir().join(format!(
        "clipboardx_vault_test_{}_{name}",
        std::process::id()
    ));
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[test]
fn vault_encrypts_at_rest_and_enforces_passphrase_envelope() {
    let dir = match temp_vault_dir("envelope") {
        Ok(d) => d,
        Err(err) => panic!("temp dir failed: {err}"),
    };
    let db_path = dir.join("vault.db");
    let store = VaultStore::new(db_path.clone());

    let secret_user = "devops_root@internal.example";
    let secret_pass = "UltraSecretPass!9876";

    if let Err(err) = store.init_with_passphrase(b"correct-horse-battery", None) {
        panic!("init_with_passphrase failed: {err}");
    }
    assert!(store.is_unlocked(VaultUnlockPolicy::OncePerSession, 10));

    let entry_id = match store.create_entry(
        &VaultEntryDraft {
            title: "Production Bastion".to_owned(),
            username: secret_user.to_owned(),
            password: secret_pass.to_owned(),
            note: "SSH gateway".to_owned(),
            auto_submit: true,
        },
        "shengchan",
        10_000,
    ) {
        Ok(id) => id,
        Err(err) => panic!("create_entry failed: {err}"),
    };

    // Lock vault cleanly; queries must fail until unlocked again.
    store.lock();
    assert!(!store.is_unlocked(VaultUnlockPolicy::OncePerSession, 10));
    assert!(store.list_entries("", 50).is_err());

    // Raw encrypted vault.db & key envelope bytes must never contain plaintext secrets.
    for path in [&db_path, &db_path.with_extension("key.json")] {
        let raw = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) => panic!("read {} failed: {err}", path.display()),
        };
        assert!(
            !raw.windows(secret_user.len())
                .any(|w| w == secret_user.as_bytes())
        );
        assert!(
            !raw.windows(secret_pass.len())
                .any(|w| w == secret_pass.as_bytes())
        );
    }

    // Wrong passphrase must fail cleanly.
    assert!(store.unlock_with_passphrase(b"wrong-pass").is_err());

    // Correct passphrase unlocks and restores entry; passphrase rotation invalidates old key.
    assert!(
        store
            .unlock_with_passphrase(b"correct-horse-battery")
            .is_ok()
    );
    let fetched = match store.get_entry(entry_id) {
        Ok(e) => e,
        Err(err) => panic!("get_entry failed: {err}"),
    };
    assert_eq!(fetched.username, secret_user);
    assert_eq!(fetched.password, secret_pass);

    assert!(
        store
            .change_passphrase(b"correct-horse-battery", b"rotated-pass-2026")
            .is_ok()
    );
    store.lock();
    assert!(
        store
            .unlock_with_passphrase(b"correct-horse-battery")
            .is_err()
    );
    assert!(store.unlock_with_passphrase(b"rotated-pass-2026").is_ok());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn trusted_device_unlock_persists_across_reopen() {
    let dir = match temp_vault_dir("trusted") {
        Ok(d) => d,
        Err(err) => panic!("temp dir failed: {err}"),
    };
    let db_path = dir.join("vault.db");
    let keyring = MemorySecretStore::new();
    let store = VaultStore::new(db_path.clone());

    assert!(store.init_or_unlock_trusted(&keyring).is_ok());
    let _id = store.create_entry(
        &VaultEntryDraft {
            title: "GitHub".to_owned(),
            username: "dev@clipboardx.io".to_owned(),
            password: "ghp_secret_value".to_owned(),
            note: "OSS repo".to_owned(),
            auto_submit: false,
        },
        "github",
        20_000,
    );

    let reopened = VaultStore::new(db_path);
    assert!(reopened.init_or_unlock_trusted(&keyring).is_ok());
    let list = match reopened.list_entries("dev", 10) {
        Ok(v) => v,
        Err(err) => panic!("list_entries failed: {err}"),
    };
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "GitHub");
    assert!(!list[0].auto_submit);

    let _ = fs::remove_dir_all(&dir);
}
