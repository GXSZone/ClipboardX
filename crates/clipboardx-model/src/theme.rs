use serde::{Deserialize, Serialize};

/// Built-in high-contrast desktop theme identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeId {
    #[default]
    ZedOneDark,
    CatppuccinMocha,
    MinimalLight,
    CatppuccinLatte,
}

impl ThemeId {
    pub const ALL: &'static [Self] = &[
        Self::ZedOneDark,
        Self::CatppuccinMocha,
        Self::MinimalLight,
        Self::CatppuccinLatte,
    ];

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::ZedOneDark => "One Dark",
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::MinimalLight => "Minimal Light",
            Self::CatppuccinLatte => "Catppuccin Latte",
        }
    }

    pub const fn is_dark(self) -> bool {
        matches!(self, Self::ZedOneDark | Self::CatppuccinMocha)
    }

    /// Single authoritative lookup for theme color tokens.
    pub const fn tokens(self) -> ThemeTokens {
        match self {
            Self::ZedOneDark => ONE_DARK_TOKENS,
            Self::MinimalLight => MINIMAL_LIGHT_TOKENS,
            Self::CatppuccinMocha => CATPPUCCIN_MOCHA_TOKENS,
            Self::CatppuccinLatte => CATPPUCCIN_LATTE_TOKENS,
        }
    }
}

/// System/manual theme selection mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    AlwaysDark,
    AlwaysLight,
}

/// 24-bit sRGB color constant with zero heap allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbHex(pub u32);

impl RgbHex {
    pub const fn r_u8(self) -> u8 {
        ((self.0 >> 16) & 0xFF) as u8
    }

    pub const fn g_u8(self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }

    pub const fn b_u8(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    pub const fn as_rgba_f32(self, alpha: f32) -> [f32; 4] {
        [
            self.r_u8() as f32 / 255.0,
            self.g_u8() as f32 / 255.0,
            self.b_u8() as f32 / 255.0,
            alpha,
        ]
    }
}

/// Borderless tonal-layering color tokens for each desktop theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeTokens {
    pub bg_window: RgbHex,
    pub bg_surface: RgbHex,
    pub bg_elevated: RgbHex,
    pub bg_hover: RgbHex,
    pub bg_selected: RgbHex,
    pub bg_input: RgbHex,
    pub bg_code: RgbHex,
    pub text_primary: RgbHex,
    pub text_secondary: RgbHex,
    pub text_muted: RgbHex,
    pub accent_blue: RgbHex,
    pub accent_blue_soft: RgbHex,
    pub accent_green: RgbHex,
    pub accent_green_soft: RgbHex,
    pub accent_amber: RgbHex,
    pub accent_amber_soft: RgbHex,
    pub accent_purple: RgbHex,
    pub accent_purple_soft: RgbHex,
    pub accent_red: RgbHex,
    pub shadow_primary_alpha: u8,
    pub shadow_tight_alpha: u8,
}

/// Color tokens for the default dark theme (`One Dark`).
pub const ONE_DARK_TOKENS: ThemeTokens = ThemeTokens {
    bg_window: RgbHex(0x14171D),
    bg_surface: RgbHex(0x1B1F27),
    bg_elevated: RgbHex(0x232833),
    bg_hover: RgbHex(0x2B313E),
    bg_selected: RgbHex(0x223452),
    bg_input: RgbHex(0x1A1E26),
    bg_code: RgbHex(0x111419),
    text_primary: RgbHex(0xF1F5F9),
    text_secondary: RgbHex(0x94A3B8),
    text_muted: RgbHex(0x64748B),
    accent_blue: RgbHex(0x3B82F6),
    accent_blue_soft: RgbHex(0x1B2F4D),
    accent_green: RgbHex(0x22C55E),
    accent_green_soft: RgbHex(0x122D21),
    accent_amber: RgbHex(0xF59E0B),
    accent_amber_soft: RgbHex(0x312512),
    accent_purple: RgbHex(0xA78BFA),
    accent_purple_soft: RgbHex(0x261E42),
    accent_red: RgbHex(0xEF4444),
    shadow_primary_alpha: 102, // 0.40 * 255
    shadow_tight_alpha: 41,    // 0.16 * 255
};

/// Color tokens for the default light theme (`Minimal Light`).
pub const MINIMAL_LIGHT_TOKENS: ThemeTokens = ThemeTokens {
    bg_window: RgbHex(0xEFF2F6),
    bg_surface: RgbHex(0xFFFFFF),
    bg_elevated: RgbHex(0xE6EAF0),
    bg_hover: RgbHex(0xDCE1EA),
    bg_selected: RgbHex(0xDDEAFF),
    bg_input: RgbHex(0xFFFFFF),
    bg_code: RgbHex(0xF3F5F9),
    text_primary: RgbHex(0x0F172A),
    text_secondary: RgbHex(0x475569),
    text_muted: RgbHex(0x7C8A9E),
    accent_blue: RgbHex(0x2563EB),
    accent_blue_soft: RgbHex(0xDBEAFE),
    accent_green: RgbHex(0x16A34A),
    accent_green_soft: RgbHex(0xDCFCE7),
    accent_amber: RgbHex(0xD97706),
    accent_amber_soft: RgbHex(0xFEF3C7),
    accent_purple: RgbHex(0x7C3AED),
    accent_purple_soft: RgbHex(0xEDE9FE),
    accent_red: RgbHex(0xDC2626),
    shadow_primary_alpha: 71, // 0.28 * 255
    shadow_tight_alpha: 26,   // 0.10 * 255
};

/// Soft dark theme: `Catppuccin Mocha`.
pub const CATPPUCCIN_MOCHA_TOKENS: ThemeTokens = ThemeTokens {
    bg_window: RgbHex(0x1E1E2E),
    bg_surface: RgbHex(0x24273A),
    bg_elevated: RgbHex(0x313244),
    bg_hover: RgbHex(0x3B3E54),
    bg_selected: RgbHex(0x363A59),
    bg_input: RgbHex(0x232638),
    bg_code: RgbHex(0x181825),
    text_primary: RgbHex(0xCDD6F4),
    text_secondary: RgbHex(0xA6ADC8),
    text_muted: RgbHex(0x6C7086),
    accent_blue: RgbHex(0x89B4FA),
    accent_blue_soft: RgbHex(0x2A324B),
    accent_green: RgbHex(0xA6E3A1),
    accent_green_soft: RgbHex(0x23382C),
    accent_amber: RgbHex(0xF9E2AF),
    accent_amber_soft: RgbHex(0x3A3222),
    accent_purple: RgbHex(0xCBA6F7),
    accent_purple_soft: RgbHex(0x352B49),
    accent_red: RgbHex(0xF38BA8),
    shadow_primary_alpha: 102,
    shadow_tight_alpha: 41,
};

/// Warm light theme: `Catppuccin Latte`.
pub const CATPPUCCIN_LATTE_TOKENS: ThemeTokens = ThemeTokens {
    bg_window: RgbHex(0xEFF1F5),
    bg_surface: RgbHex(0xFFFFFF),
    bg_elevated: RgbHex(0xE6E9EF),
    bg_hover: RgbHex(0xDCE0E8),
    bg_selected: RgbHex(0xE4DDFB),
    bg_input: RgbHex(0xFFFFFF),
    bg_code: RgbHex(0xF2F4F8),
    text_primary: RgbHex(0x4C4F69),
    text_secondary: RgbHex(0x5C5F77),
    text_muted: RgbHex(0x8C8FA1),
    accent_blue: RgbHex(0x1E66F5),
    accent_blue_soft: RgbHex(0xDCE6FD),
    accent_green: RgbHex(0x40A02B),
    accent_green_soft: RgbHex(0xDDF4D9),
    accent_amber: RgbHex(0xDF8E1D),
    accent_amber_soft: RgbHex(0xFCECD4),
    accent_purple: RgbHex(0x8839EF),
    accent_purple_soft: RgbHex(0xEDE0FD),
    accent_red: RgbHex(0xD20F39),
    shadow_primary_alpha: 71,
    shadow_tight_alpha: 26,
};

/// Shared pixel dimensions and timing constants for the desktop UI.
pub struct LayoutSpec;

impl LayoutSpec {
    pub const WINDOW_DUAL_WIDTH: f32 = 680.0;
    pub const WINDOW_SLIM_WIDTH: f32 = 328.0;
    pub const WINDOW_HEIGHT: f32 = 420.0;
    pub const WINDOW_RADIUS: f32 = 14.0;

    pub const TOPBAR_DUAL_HEIGHT: f32 = 38.0;
    pub const TOPBAR_SLIM_HEIGHT: f32 = 36.0;
    pub const FILTER_STRIP_HEIGHT: f32 = 29.0;
    pub const FOOTER_HEIGHT: f32 = 27.0;

    pub const LEFT_LIST_WIDTH: f32 = 246.0;
    pub const RIGHT_PREVIEW_WIDTH: f32 = 410.0;
    pub const RIGHT_SURFACE_RADIUS: f32 = 10.0;

    pub const LIST_ROW_HEIGHT: f32 = 40.0;
    pub const LIST_ROW_GAP: f32 = 2.0;
    pub const IMAGE_ROW_HEIGHT: f32 = 82.0;
    pub const IMAGE_THUMB_HEIGHT: f32 = 70.0;
    pub const LIST_ROW_RADIUS: f32 = 8.0;
    pub const TYPE_BADGE_SIZE: f32 = 22.0;
    pub const TYPE_BADGE_RADIUS: f32 = 6.0;

    pub const SETTINGS_NAV_WIDTH: f32 = 164.0;
    pub const SETTINGS_CARD_RADIUS: f32 = 9.0;

    pub const COLLAPSE_DURATION_MS: u64 = 180;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_themes_have_distinct_surface_hierarchy() {
        for id in ThemeId::ALL {
            let t = id.tokens();
            assert_ne!(t.bg_window, t.bg_surface);
            assert_ne!(t.bg_surface, t.bg_elevated);
            assert_ne!(t.text_primary, t.bg_surface);
        }
    }
}
