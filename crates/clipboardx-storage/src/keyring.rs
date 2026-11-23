use clipboardx_model::{SecretStore, StorageError};
use keyring::Entry;
use rand::Rng;
use zeroize::Zeroizing;

const KEYRING_SERVICE: &str = "io.clipboardx.app";
const MASTER_KEY_ACCOUNT: &str = "master_key_v1";
const VAULT_DEK_ACCOUNT: &str = "vault_trusted_dek_v1";

/// Production `SecretStore` backed by the OS credential store
/// (Windows Credential Manager, macOS Keychain, or Linux Secret Service).
#[derive(Debug, Default)]
pub struct OsSecretStore;

impl OsSecretStore {
    pub const fn new() -> Self {
        Self
    }

    fn entry(account: &str) -> Result<Entry, StorageError> {
        Entry::new(KEYRING_SERVICE, account)
            .map_err(|err| StorageError::Keyring(format!("create keyring entry failed: {err}")))
    }
}

impl SecretStore for OsSecretStore {
    fn get_or_create_master_key(&self) -> Result<Zeroizing<Vec<u8>>, StorageError> {
        let entry = Self::entry(MASTER_KEY_ACCOUNT)?;
        match entry.get_secret() {
            Ok(secret) if secret.len() == 32 => Ok(Zeroizing::new(secret)),
            Ok(secret) if !secret.is_empty() => {
                // Decode hex-encoded keys stored by legacy or string-only keyring backends.
                if let Ok(decoded) = hex::decode(&secret)
                    && decoded.len() == 32
                {
                    return Ok(Zeroizing::new(decoded));
                }
                Err(StorageError::Keyring(
                    "stored master key has unexpected length".to_owned(),
                ))
            }
            Ok(_) | Err(keyring::Error::NoEntry) => {
                let mut key = Zeroizing::new(vec![0u8; 32]);
                rand::rng().fill_bytes(&mut key);
                entry.set_secret(&key).map_err(|err| {
                    StorageError::Keyring(format!("persist master key failed: {err}"))
                })?;
                Ok(key)
            }
            Err(err) => Err(StorageError::Keyring(format!(
                "read master key failed: {err}"
            ))),
        }
    }

    fn save_vault_trusted_dek(&self, dek: &[u8]) -> Result<(), StorageError> {
        let entry = Self::entry(VAULT_DEK_ACCOUNT)?;
        entry
            .set_secret(dek)
            .map_err(|err| StorageError::Keyring(format!("save trusted vault DEK failed: {err}")))
    }

    fn load_vault_trusted_dek(&self) -> Result<Option<Zeroizing<Vec<u8>>>, StorageError> {
        let entry = Self::entry(VAULT_DEK_ACCOUNT)?;
        match entry.get_secret() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(StorageError::Keyring(format!(
                "load trusted vault DEK failed: {err}"
            ))),
        }
    }

    fn clear_vault_trusted_dek(&self) -> Result<(), StorageError> {
        let entry = Self::entry(VAULT_DEK_ACCOUNT)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(StorageError::Keyring(format!(
                "clear trusted vault DEK failed: {err}"
            ))),
        }
    }
}
