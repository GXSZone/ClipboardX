use serde::{Deserialize, Serialize};

/// In-panel keyboard actions (SSOT driving both GPUI bindings and footer legend).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShortcutAction {
    PasteOrAutofill,
    PastePlainOrPasswordOnly,
    QuickPasteIndex(u8), // 1..=9
    TogglePin,
    MoveToGroup,
    EditAlias,
    NewVaultEntry,
    ToggleCompactMode,
    DeleteItem,
    HideOrClearSearch,
    SelectPrev,
    SelectNext,
}

/// Footer shortcut legend entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FooterHintSpec {
    pub keycap: &'static str,
    pub label_i18n_key: &'static str,
}

pub const FOOTER_HINTS_DUAL: &[FooterHintSpec] = &[
    FooterHintSpec {
        keycap: "↵",
        label_i18n_key: "footer.paste",
    },
    FooterHintSpec {
        keycap: "⇧↵",
        label_i18n_key: "footer.paste_plain",
    },
    FooterHintSpec {
        keycap: "Ctrl+1~9",
        label_i18n_key: "footer.quick_paste",
    },
    FooterHintSpec {
        keycap: "Ctrl+\\",
        label_i18n_key: "footer.toggle_compact",
    },
    FooterHintSpec {
        keycap: "Esc",
        label_i18n_key: "footer.hide",
    },
];

pub const FOOTER_HINTS_SLIM: &[FooterHintSpec] = &[
    FooterHintSpec {
        keycap: "↵",
        label_i18n_key: "footer.paste",
    },
    FooterHintSpec {
        keycap: "⇧↵",
        label_i18n_key: "footer.paste_plain",
    },
    FooterHintSpec {
        keycap: "Ctrl+\\",
        label_i18n_key: "footer.expand",
    },
];
