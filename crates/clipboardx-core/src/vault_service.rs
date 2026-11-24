use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use clipboardx_model::{
    ClipboardXError, SecretStore, VaultEntry, VaultEntryDraft, VaultEntryId, VaultUnlockPolicy,
};
use clipboardx_storage::VaultStore;

use crate::pinyin_index::build_pinyin_index;

/// Orchestrates encrypted `vault.db` access, OS keyring trusted caching,
/// passphrase lifecycle, and pinyin-indexed credential search.
#[derive(Clone)]
pub struct VaultService {
    store: VaultStore,
    secret_store: Arc<dyn SecretStore>,
}

impl VaultService {
    pub fn new(store: VaultStore, secret_store: Arc<dyn SecretStore>) -> Self {
        Self {
            store,
            secret_store,
        }
    }

    pub fn exists(&self) -> bool {
        self.store.exists()
    }

    pub fn has_passphrase_envelope(&self) -> bool {
        self.store.has_passphrase_envelope()
    }

    pub fn is_unlocked(&self, policy: VaultUnlockPolicy, timeout_minutes: u32) -> bool {
        self.store.is_unlocked(policy, timeout_minutes)
    }

    pub fn lock(&self) {
        self.store.lock();
    }

    pub fn shrink_memory(&self) {
        self.store.shrink_memory();
    }

    /// Attempts silent unlock when policy is `TrustedDeviceNoPrompt`.
    /// Returns `Ok(true)` if unlocked or newly initialized silently, `Ok(false)` if a passphrase is required.
    pub fn try_unlock_trusted(&self, policy: VaultUnlockPolicy) -> Result<bool, ClipboardXError> {
        if policy != VaultUnlockPolicy::TrustedDeviceNoPrompt {
            return Ok(false);
        }
        match self.store.init_or_unlock_trusted(&*self.secret_store) {
            Ok(()) => Ok(true),
            Err(clipboardx_model::StorageError::VaultLocked) => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    /// Initializes a brand-new vault with a master passphrase, or seamlessly attaches a
    /// passphrase envelope to an existing trusted-keyring-only `vault.db` using its original DEK.
    pub fn init_with_passphrase(
        &self,
        passphrase: &[u8],
        cache_trusted: bool,
    ) -> Result<(), ClipboardXError> {
        if self.store.exists() && !self.store.has_passphrase_envelope() {
            self.store.attach_passphrase_via_trusted(
                passphrase,
                &*self.secret_store,
                cache_trusted,
            )?;
            return Ok(());
        }
        let store_opt = cache_trusted.then_some(&*self.secret_store);
        self.store.init_with_passphrase(passphrase, store_opt)?;
        Ok(())
    }

    /// Resets an unrecoverable/key-mismatched vault database and envelope so it can be recreated.
    pub fn reset_vault(&self) -> Result<(), ClipboardXError> {
        self.store.reset_vault(&*self.secret_store)?;
        Ok(())
    }

    /// Unlocks an existing passphrase-sealed vault, optionally caching trusted DEK afterward.
    pub fn unlock_with_passphrase(
        &self,
        passphrase: &[u8],
        cache_trusted: bool,
    ) -> Result<(), ClipboardXError> {
        self.store.unlock_with_passphrase(passphrase)?;
        if cache_trusted {
            self.store.enable_trusted_cache(&*self.secret_store)?;
        }
        Ok(())
    }

    /// Enables trusted passwordless unlock for an already unlocked vault session.
    pub fn enable_trusted_while_unlocked(&self) -> Result<(), ClipboardXError> {
        self.store.enable_trusted_cache(&*self.secret_store)?;
        Ok(())
    }

    /// Clears trusted DEK from the OS keyring when switching back to passphrase prompt policies.
    pub fn clear_trusted_cache(&self) -> Result<(), ClipboardXError> {
        self.secret_store.clear_vault_trusted_dek()?;
        Ok(())
    }

    pub fn change_passphrase(&self, current: &[u8], next: &[u8]) -> Result<(), ClipboardXError> {
        self.store.change_passphrase(current, next)?;
        Ok(())
    }

    pub fn attach_passphrase_while_unlocked(&self, next: &[u8]) -> Result<(), ClipboardXError> {
        self.store.attach_passphrase_while_unlocked(next)?;
        Ok(())
    }

    pub fn list_entries(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<VaultEntry>, ClipboardXError> {
        Ok(self.store.list_entries(query, limit)?)
    }

    pub fn get_entry(&self, id: VaultEntryId) -> Result<VaultEntry, ClipboardXError> {
        Ok(self.store.get_entry(id)?)
    }

    pub fn create_entry(&self, draft: &VaultEntryDraft) -> Result<VaultEntryId, ClipboardXError> {
        let pinyin_idx = build_pinyin_index(&format!("{} {}", draft.title, draft.note));
        Ok(self.store.create_entry(draft, &pinyin_idx, now_ms())?)
    }

    pub fn update_entry(
        &self,
        id: VaultEntryId,
        draft: &VaultEntryDraft,
    ) -> Result<(), ClipboardXError> {
        let pinyin_idx = build_pinyin_index(&format!("{} {}", draft.title, draft.note));
        Ok(self.store.update_entry(id, draft, &pinyin_idx, now_ms())?)
    }

    pub fn toggle_auto_submit(&self, id: VaultEntryId) -> Result<bool, ClipboardXError> {
        Ok(self.store.toggle_auto_submit(id, now_ms())?)
    }

    pub fn delete_entry(&self, id: VaultEntryId) -> Result<(), ClipboardXError> {
        Ok(self.store.delete_entry(id)?)
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
