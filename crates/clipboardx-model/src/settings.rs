use serde::{Deserialize, Serialize};

use crate::{ThemeId, ThemeMode};

/// Supported UI locales.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Locale {
    #[default]
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en-US")]
    EnUs,
}

impl Locale {
    pub const fn code(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::EnUs => "en-US",
        }
    }
}

/// Global hotkey activation mode (Toggle / Hold-to-Peek / Smart Hybrid).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyTriggerMode {
    /// Press < 260ms toggles persistent visibility; hold >= 260ms hides on key release.
    #[default]
    SmartHybrid,
    /// Stays visible only while holding the hotkey modifier, hides immediately on release.
    HoldToPeek,
    /// Press once to show, press again (or Esc) to hide.
    Toggle,
}

/// Default action when pressing Enter on a selected item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PasteActionMode {
    #[default]
    AutoPaste,
    CopyOnly,
}

/// Behavior when the floating panel loses focus to another desktop window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowBlurBehavior {
    /// Collapse into the compact right-edge floating mini capsule (38x58) on blur,
    /// peeking the quick-paste card on hover and expanding on click/focus (default).
    #[default]
    DockRightCapsule,
    /// Hide completely to the system tray whenever focus is lost.
    AutoHide,
}

/// Unlock policy for the separate encrypted account vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VaultUnlockPolicy {
    /// Store the vault DEK in the OS keyring for passwordless local access.
    TrustedDeviceNoPrompt,
    /// Require master password or OS biometrics, cached for `vault_timeout_minutes`.
    #[default]
    PromptWithTimeout,
    /// Prompt once per application session.
    OncePerSession,
}

/// Release channel for background auto-update checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
}

/// Borderless settings window navigation sections (SSOT).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SettingsTab {
    #[default]
    General,
    Appearance,
    VaultPrivacy,
    StorageBackup,
    OcrPlugins,
    AboutUpdates,
}

impl SettingsTab {
    pub const ALL: &'static [Self] = &[
        Self::General,
        Self::Appearance,
        Self::VaultPrivacy,
        Self::StorageBackup,
        Self::OcrPlugins,
        Self::AboutUpdates,
    ];

    pub const fn i18n_key(self) -> &'static str {
        match self {
            Self::General => "settings.nav.general",
            Self::Appearance => "settings.nav.appearance",
            Self::VaultPrivacy => "settings.nav.vault_privacy",
            Self::StorageBackup => "settings.nav.storage_backup",
            Self::OcrPlugins => "settings.nav.ocr_plugins",
            Self::AboutUpdates => "settings.nav.about_updates",
        }
    }
}

/// Retention and size bounds for clipboard history pruning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub max_days: u32,
    pub max_items: u32,
    pub max_item_bytes: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_days: 30,
            max_items: 2000,
            max_item_bytes: 20 * 1024 * 1024, // 20 MB
        }
    }
}

/// Persistent application configuration schema (SSOT).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub locale: Locale,
    pub toggle_hotkey: String,
    pub compact_hotkey: String,
    pub pause_hotkey: String,
    pub hotkey_trigger_mode: HotkeyTriggerMode,
    pub hold_threshold_ms: u64,
    pub paste_mode: PasteActionMode,
    #[serde(default)]
    pub blur_behavior: WindowBlurBehavior,
    pub launch_at_startup: bool,
    pub remember_compact_mode: bool,
    pub is_compact_mode: bool,
    pub smooth_collapse_animation: bool,
    pub theme_mode: ThemeMode,
    pub dark_theme: ThemeId,
    pub light_theme: ThemeId,
    pub window_vibrancy: bool,
    pub vault_unlock_policy: VaultUnlockPolicy,
    pub vault_timeout_minutes: u32,
    pub ignore_confidential_clipboard: bool,
    pub restore_clipboard_after_autofill: bool,
    pub app_blacklist: Vec<String>,
    pub retention: RetentionPolicy,
    pub auto_ocr_native: bool,
    pub active_ocr_plugin: Option<String>,
    pub background_auto_update: bool,
    pub update_channel: UpdateChannel,
}

impl Default for AppConfig {
    fn default() -> Self {
        let toggle_hotkey = if cfg!(target_os = "macos") {
            "Cmd+Shift+V".to_owned()
        } else {
            "Alt+V".to_owned()
        };

        Self {
            locale: Locale::ZhCn,
            toggle_hotkey,
            compact_hotkey: "Ctrl+\\".to_owned(),
            pause_hotkey: "Ctrl+Alt+P".to_owned(),
            hotkey_trigger_mode: HotkeyTriggerMode::SmartHybrid,
            hold_threshold_ms: 260,
            paste_mode: PasteActionMode::AutoPaste,
            blur_behavior: WindowBlurBehavior::DockRightCapsule,
            launch_at_startup: false,
            remember_compact_mode: true,
            is_compact_mode: false,
            smooth_collapse_animation: true,
            theme_mode: ThemeMode::System,
            dark_theme: ThemeId::ZedOneDark,
            light_theme: ThemeId::MinimalLight,
            window_vibrancy: false,
            vault_unlock_policy: VaultUnlockPolicy::PromptWithTimeout,
            vault_timeout_minutes: 10,
            ignore_confidential_clipboard: true,
            restore_clipboard_after_autofill: true,
            app_blacklist: vec![
                "1Password".to_owned(),
                "Bitwarden".to_owned(),
                "KeePassXC".to_owned(),
                "Enpass".to_owned(),
                "Keychain Access".to_owned(),
            ],
            retention: RetentionPolicy::default(),
            auto_ocr_native: true,
            active_ocr_plugin: None,
            background_auto_update: true,
            update_channel: UpdateChannel::Stable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid_and_safe() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.retention.max_days, 30);
        assert_eq!(cfg.retention.max_items, 2000);
        assert!(cfg.ignore_confidential_clipboard);
        assert!(cfg.restore_clipboard_after_autofill);
        assert!(!cfg.window_vibrancy);
    }
}
