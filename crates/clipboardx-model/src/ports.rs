use zeroize::Zeroizing;

use crate::{
    CapturedClipboardEvent, ClipboardPayload, OcrError, PlatformError, StorageError, UpdaterError,
};

/// Opaque handle representing the previously focused OS window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForegroundWindowToken(pub isize);

/// Global hotkey events dispatched from OS listeners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEventKind {
    TogglePanelPressed,
    ToggleCompactPressed,
    TogglePausePressed,
}

/// Multi-platform OS bridge port (Windows / macOS / Linux X11 & Wayland).
pub trait PlatformBridge: Send + Sync {
    /// Captures a plain-text clipboard snapshot prior to autofill injection.
    fn read_text_snapshot(&self) -> Result<Option<String>, PlatformError>;

    /// Writes payload to the OS clipboard while suppressing self-capture loops.
    fn write_clipboard(&self, payload: &ClipboardPayload) -> Result<(), PlatformError>;

    /// Clears the OS clipboard.
    fn clear_clipboard(&self) -> Result<(), PlatformError>;

    /// Records the currently focused external window handle.
    fn capture_foreground_window(&self) -> Option<ForegroundWindowToken>;

    /// Restores focus back to the recorded external window.
    fn restore_foreground_window(&self, token: ForegroundWindowToken) -> Result<(), PlatformError>;

    /// Simulates the platform paste shortcut (Ctrl+V / Cmd+V).
    fn simulate_paste(&self) -> Result<(), PlatformError>;

    /// Simulates a single control key tap (Tab / Enter) during account autofill.
    fn simulate_key_tap(&self, key: SimulatedKey) -> Result<(), PlatformError>;

    /// Polls whether the toggle hotkey's modifier key is still held down.
    fn is_hotkey_modifier_held(&self, hotkey_spec: &str) -> bool;

    /// Returns true on restricted Wayland compositors requiring copy-only fallback.
    fn is_restricted_wayland(&self) -> bool;
}

/// Control keys used by the account autofill sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulatedKey {
    Tab,
    Enter,
}

/// OS secret storage port (supports real OS keyrings and in-memory test doubles).
pub trait SecretStore: Send + Sync {
    fn get_or_create_master_key(&self) -> Result<Zeroizing<Vec<u8>>, StorageError>;
    fn save_vault_trusted_dek(&self, dek: &[u8]) -> Result<(), StorageError>;
    fn load_vault_trusted_dek(&self) -> Result<Option<Zeroizing<Vec<u8>>>, StorageError>;
    fn clear_vault_trusted_dek(&self) -> Result<(), StorageError>;
}

/// Recognized text payload returned by an OCR provider.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrOutput {
    pub full_text: String,
    pub provider_id: String,
}

/// Pluggable OCR backend port (WinRT / macOS Vision / external JSON-RPC sidecar).
pub trait OcrProvider: Send + Sync {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn is_available(&self) -> bool;
    fn recognize_png(
        &self,
        png_bytes: &[u8],
        lang_hint: Option<&str>,
    ) -> Result<OcrOutput, OcrError>;
}

/// Metadata for an available signed release update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReleaseInfo {
    pub version: String,
    pub notes: String,
    pub download_url: String,
    pub sha256_hex: String,
    pub signature_hex: String,
}

/// Background updater client port.
pub trait UpdateClient: Send + Sync {
    fn check_latest(
        &self,
        channel: crate::UpdateChannel,
    ) -> Result<Option<UpdateReleaseInfo>, UpdaterError>;
}

/// Callback invoked when the clipboard monitor captures a valid non-confidential item.
pub type ClipboardEventHandler = Box<dyn Fn(CapturedClipboardEvent) + Send + Sync>;
