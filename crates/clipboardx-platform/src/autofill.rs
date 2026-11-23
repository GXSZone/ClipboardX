#![forbid(unsafe_code)]

use std::thread;
use std::time::Duration;

use clipboardx_model::{
    ClipboardPayload, ClipboardXError, ForegroundWindowToken, PlatformBridge, SimulatedKey,
    VaultEntry,
};

use crate::privacy::SelfWriteSuppressor;

/// Credential injection mode triggered from the vault UI or shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutofillAction {
    /// Paste username -> Tab -> paste password (-> optional Enter).
    FullSequence,
    /// Paste only the username.
    UsernameOnly,
    /// Paste only the password (e.g. two-step login flows / `Shift+Enter`).
    PasswordOnly,
}

/// RAII guard that snapshots the OS plain-text clipboard before credential autofill,
/// silences self-capture during credential writes, and restores the prior clipboard
/// on scope exit so secret strings never persist on the clipboard or enter `history.db`.
pub struct ClipboardRestoreGuard<'a> {
    platform: &'a dyn PlatformBridge,
    suppressor: &'a SelfWriteSuppressor,
    previous_text: Option<String>,
    restore_enabled: bool,
}

impl<'a> ClipboardRestoreGuard<'a> {
    pub fn enter(
        platform: &'a dyn PlatformBridge,
        suppressor: &'a SelfWriteSuppressor,
        restore_enabled: bool,
    ) -> Self {
        let previous_text = if restore_enabled {
            platform.read_text_snapshot().ok().flatten()
        } else {
            None
        };
        suppressor.suppress_all_for(Duration::from_millis(1200));
        Self {
            platform,
            suppressor,
            previous_text,
            restore_enabled,
        }
    }
}

impl Drop for ClipboardRestoreGuard<'_> {
    fn drop(&mut self) {
        if !self.restore_enabled {
            return;
        }
        // Wait for the target application to finish consuming the last simulated paste.
        thread::sleep(Duration::from_millis(150));
        self.suppressor.suppress_all_for(Duration::from_millis(600));
        match &self.previous_text {
            Some(prev) => {
                let payload = ClipboardPayload::Text {
                    plain: prev.clone(),
                };
                if let Err(err) = self.platform.write_clipboard(&payload) {
                    tracing::warn!(error = %err, "failed to restore previous clipboard after autofill");
                }
            }
            None => {
                let _ = self.platform.clear_clipboard();
            }
        }
    }
}

/// Executes credential autofill against the previously active external window under
/// `ClipboardRestoreGuard` protection.
pub fn execute_account_autofill(
    platform: &dyn PlatformBridge,
    suppressor: &SelfWriteSuppressor,
    foreground: Option<ForegroundWindowToken>,
    entry: &VaultEntry,
    action: AutofillAction,
    restore_clipboard: bool,
) -> Result<(), ClipboardXError> {
    if platform.is_restricted_wayland() {
        let text = match action {
            AutofillAction::UsernameOnly => entry.username.clone(),
            AutofillAction::FullSequence | AutofillAction::PasswordOnly => entry.password.clone(),
        };
        suppressor.suppress_all_for(Duration::from_millis(800));
        platform.write_clipboard(&ClipboardPayload::Text { plain: text })?;
        return Ok(());
    }

    let _guard = ClipboardRestoreGuard::enter(platform, suppressor, restore_clipboard);

    if let Some(target) = foreground {
        let _ = platform.restore_foreground_window(target);
        thread::sleep(Duration::from_millis(75));
    }

    match action {
        AutofillAction::UsernameOnly => {
            platform.write_clipboard(&ClipboardPayload::Text {
                plain: entry.username.clone(),
            })?;
            thread::sleep(Duration::from_millis(35));
            platform.simulate_paste()?;
        }
        AutofillAction::PasswordOnly => {
            platform.write_clipboard(&ClipboardPayload::Text {
                plain: entry.password.clone(),
            })?;
            thread::sleep(Duration::from_millis(35));
            platform.simulate_paste()?;
        }
        AutofillAction::FullSequence => {
            platform.write_clipboard(&ClipboardPayload::Text {
                plain: entry.username.clone(),
            })?;
            thread::sleep(Duration::from_millis(35));
            platform.simulate_paste()?;

            thread::sleep(Duration::from_millis(85));
            platform.simulate_key_tap(SimulatedKey::Tab)?;
            thread::sleep(Duration::from_millis(65));

            platform.write_clipboard(&ClipboardPayload::Text {
                plain: entry.password.clone(),
            })?;
            thread::sleep(Duration::from_millis(35));
            platform.simulate_paste()?;

            if entry.auto_submit {
                thread::sleep(Duration::from_millis(80));
                platform.simulate_key_tap(SimulatedKey::Enter)?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use clipboardx_model::PlatformError;

    use super::*;

    struct FakeBridge {
        clipboard: Mutex<Option<String>>,
        ops: Mutex<Vec<String>>,
    }

    impl FakeBridge {
        fn new(initial: Option<&str>) -> Self {
            Self {
                clipboard: Mutex::new(initial.map(str::to_owned)),
                ops: Mutex::new(Vec::new()),
            }
        }
    }

    impl PlatformBridge for FakeBridge {
        fn read_text_snapshot(&self) -> Result<Option<String>, PlatformError> {
            Ok(self.clipboard.lock().ok().and_then(|g| g.clone()))
        }

        fn write_clipboard(&self, payload: &ClipboardPayload) -> Result<(), PlatformError> {
            if let ClipboardPayload::Text { plain } = payload {
                if let Ok(mut g) = self.clipboard.lock() {
                    *g = Some(plain.clone());
                }
                if let Ok(mut ops) = self.ops.lock() {
                    ops.push(format!("write:{plain}"));
                }
            }
            Ok(())
        }

        fn clear_clipboard(&self) -> Result<(), PlatformError> {
            if let Ok(mut g) = self.clipboard.lock() {
                *g = None;
            }
            if let Ok(mut ops) = self.ops.lock() {
                ops.push("clear".to_owned());
            }
            Ok(())
        }

        fn capture_foreground_window(&self) -> Option<ForegroundWindowToken> {
            None
        }

        fn restore_foreground_window(&self, _: ForegroundWindowToken) -> Result<(), PlatformError> {
            if let Ok(mut ops) = self.ops.lock() {
                ops.push("focus".to_owned());
            }
            Ok(())
        }

        fn simulate_paste(&self) -> Result<(), PlatformError> {
            if let Ok(mut ops) = self.ops.lock() {
                ops.push("paste".to_owned());
            }
            Ok(())
        }

        fn simulate_key_tap(&self, key: SimulatedKey) -> Result<(), PlatformError> {
            if let Ok(mut ops) = self.ops.lock() {
                ops.push(format!("key:{key:?}"));
            }
            Ok(())
        }

        fn is_hotkey_modifier_held(&self, _: &str) -> bool {
            false
        }

        fn is_restricted_wayland(&self) -> bool {
            false
        }
    }

    #[test]
    fn autofill_runs_full_sequence_and_restores_prior_clipboard() {
        let bridge = FakeBridge::new(Some("original clipboard text"));
        let suppressor = SelfWriteSuppressor::new();
        let entry = VaultEntry {
            id: clipboardx_model::VaultEntryId(1),
            title: "GitHub".to_owned(),
            username: "dev@clipboardx.io".to_owned(),
            password: "s3cr3t-password".to_owned(),
            note: String::new(),
            auto_submit: true,
            pinyin_index: String::new(),
            updated_at_ms: 0,
        };

        let res = execute_account_autofill(
            &bridge,
            &suppressor,
            Some(ForegroundWindowToken(99)),
            &entry,
            AutofillAction::FullSequence,
            true,
        );
        assert!(res.is_ok());

        // Original clipboard text must be restored without password leakage.
        assert_eq!(
            bridge.read_text_snapshot().ok().flatten().as_deref(),
            Some("original clipboard text")
        );
        // Any self-write must remain suppressed right after drop.
        assert!(suppressor.should_ignore("arbitrary-hash"));

        let ops = bridge.ops.lock().map(|g| g.clone()).unwrap_or_default();
        assert_eq!(
            ops,
            vec![
                "focus",
                "write:dev@clipboardx.io",
                "paste",
                "key:Tab",
                "write:s3cr3t-password",
                "paste",
                "key:Enter",
                "write:original clipboard text",
            ]
        );
    }
}
