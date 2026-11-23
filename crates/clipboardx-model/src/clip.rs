use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Strongly typed clipboard history item ID to prevent raw integer mix-ups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClipItemId(pub i64);

/// Strongly typed custom group ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupId(pub i64);

/// Strongly typed account vault entry ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VaultEntryId(pub i64);

impl fmt::Display for ClipItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "clip:{}", self.0)
    }
}

impl fmt::Display for GroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "group:{}", self.0)
    }
}

impl fmt::Display for VaultEntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "vault:{}", self.0)
    }
}

/// Primary clipboard payload category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipKind {
    Text,
    RichText,
    Image,
    Files,
}

impl ClipKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::RichText => "rich_text",
            Self::Image => "image",
            Self::Files => "files",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "text" => Some(Self::Text),
            "rich_text" => Some(Self::RichText),
            "image" => Some(Self::Image),
            "files" => Some(Self::Files),
            _ => None,
        }
    }
}

/// Geek smart-detected content subtype assigned by `clipboardx-core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SmartSubtype {
    #[default]
    Plain,
    Color,
    Url,
    Json,
    Code,
    Timestamp,
    Base64,
    Jwt,
}

impl SmartSubtype {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Color => "color",
            Self::Url => "url",
            Self::Json => "json",
            Self::Code => "code",
            Self::Timestamp => "timestamp",
            Self::Base64 => "base64",
            Self::Jwt => "jwt",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "color" => Self::Color,
            "url" => Self::Url,
            "json" => Self::Json,
            "code" => Self::Code,
            "timestamp" => Self::Timestamp,
            "base64" => Self::Base64,
            "jwt" => Self::Jwt,
            _ => Self::Plain,
        }
    }

    /// Short badge label rendered inside the 22x22 left icon tile.
    pub const fn badge_glyph(self, kind: ClipKind) -> &'static str {
        match kind {
            ClipKind::Image => "IMG",
            ClipKind::Files => "DIR",
            ClipKind::RichText => "RTF",
            ClipKind::Text => match self {
                Self::Plain => "TXT",
                Self::Color => "CLR",
                Self::Url => "URL",
                Self::Json => "{}",
                Self::Code => "</>",
                Self::Timestamp => "TS",
                Self::Base64 => "B64",
                Self::Jwt => "JWT",
            },
        }
    }
}

/// Top filter strip tabs (SSOT driving dual-pane, slim-pane, and search query prefixes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FilterTab {
    #[default]
    All,
    Accounts,
    Text,
    Image,
    Code,
    Color,
    Files,
    Pinned,
}

impl FilterTab {
    /// Full filter tab list for the dual-pane layout.
    pub const ALL_FULL: &'static [Self] = &[
        Self::All,
        Self::Pinned,
        Self::Accounts,
        Self::Text,
        Self::Image,
        Self::Code,
        Self::Color,
        Self::Files,
    ];

    /// Compact filter tab list for the slim and default tab strip.
    pub const ALL_SLIM: &'static [Self] = &[
        Self::All,
        Self::Pinned,
        Self::Accounts,
        Self::Text,
        Self::Image,
        Self::Code,
    ];

    pub const fn i18n_key(self) -> &'static str {
        match self {
            Self::All => "filter.all",
            Self::Accounts => "filter.accounts",
            Self::Text => "filter.text",
            Self::Image => "filter.image",
            Self::Code => "filter.code",
            Self::Color => "filter.color",
            Self::Files => "filter.files",
            Self::Pinned => "filter.pinned",
        }
    }
}

/// Raw payload captured from the OS clipboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardPayload {
    Text {
        plain: String,
    },
    RichText {
        plain: String,
        html: Option<String>,
        rtf: Option<String>,
    },
    Image {
        png_bytes: Vec<u8>,
        width: u32,
        height: u32,
    },
    Files {
        paths: Vec<String>,
    },
}

impl ClipboardPayload {
    pub const fn kind(&self) -> ClipKind {
        match self {
            Self::Text { .. } => ClipKind::Text,
            Self::RichText { .. } => ClipKind::RichText,
            Self::Image { .. } => ClipKind::Image,
            Self::Files { .. } => ClipKind::Files,
        }
    }
}

/// Captured clipboard event with source application metadata and timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedClipboardEvent {
    pub payload: ClipboardPayload,
    pub source_app_name: Option<String>,
    pub source_app_id: Option<String>,
    pub captured_at_ms: i64,
}

/// Custom collection group; grouped items are exempt from automatic retention eviction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub color: Option<String>,
    pub sort_order: i32,
    pub created_at_ms: i64,
}

/// Lightweight history item projection used for 40px virtualized list rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipSummary {
    pub id: ClipItemId,
    pub content_hash: String,
    pub kind: ClipKind,
    pub subtype: SmartSubtype,
    pub detected_lang: Option<String>,
    pub preview_text: String,
    pub alias: String,
    pub ocr_text: String,
    pub thumb_ref: Option<String>,
    pub source_app_name: Option<String>,
    pub is_pinned: bool,
    pub group_id: Option<GroupId>,
    pub use_count: u32,
    pub byte_size: u64,
    pub meta_json: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Full history item loaded on demand for right-pane preview and paste write-back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipItem {
    pub summary: ClipSummary,
    pub full_text: String,
    pub payload_inline: Option<String>,
    pub blob_ref: Option<String>,
}

/// Lightweight account credential card stored in the separate encrypted `vault.db`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct VaultEntry {
    #[zeroize(skip)]
    pub id: VaultEntryId,
    pub title: String,
    pub username: String,
    pub password: String,
    pub note: String,
    pub auto_submit: bool,
    pub pinyin_index: String,
    #[zeroize(skip)]
    pub updated_at_ms: i64,
}

/// Input draft for creating or updating a vault entry.
#[derive(Debug, Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct VaultEntryDraft {
    pub title: String,
    pub username: String,
    pub password: String,
    pub note: String,
    pub auto_submit: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_kind_roundtrip() {
        for kind in [
            ClipKind::Text,
            ClipKind::RichText,
            ClipKind::Image,
            ClipKind::Files,
        ] {
            assert_eq!(ClipKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ClipKind::parse("unknown"), None);
    }

    #[test]
    fn filter_tabs_cover_full_and_slim_modes() {
        assert_eq!(FilterTab::ALL_FULL.len(), 8);
        assert_eq!(FilterTab::ALL_SLIM.len(), 6);
        assert_eq!(FilterTab::ALL_FULL[1], FilterTab::Pinned);
        assert_eq!(FilterTab::ALL_SLIM[1], FilterTab::Pinned);
    }
}
