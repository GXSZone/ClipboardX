use std::borrow::Cow;

use clipboardx_model::{AppConfig, RgbHex, ThemeId, ThemeMode, ThemeTokens};
use gpui_kit::component::{Theme as ComponentTheme, ThemeMode as ComponentThemeMode};
use gpui_kit::{App, BoxShadow, FontWeight, Hsla, Rgba, Window, WindowAppearance, point, px};

/// Embedded Fira Code static font weights for monospace rendering.
const FIRA_CODE_REGULAR: &[u8] = include_bytes!("../../../../assets/fonts/FiraCode-Regular.ttf");
const FIRA_CODE_MEDIUM: &[u8] = include_bytes!("../../../../assets/fonts/FiraCode-Medium.ttf");
const FIRA_CODE_BOLD: &[u8] = include_bytes!("../../../../assets/fonts/FiraCode-Bold.ttf");

/// Registers embedded font faces into the GPUI text system at application startup.
pub fn register_embedded_fonts(cx: &App) {
    let fonts = vec![
        Cow::Borrowed(FIRA_CODE_REGULAR),
        Cow::Borrowed(FIRA_CODE_MEDIUM),
        Cow::Borrowed(FIRA_CODE_BOLD),
    ];
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        tracing::warn!(error = %err, "failed to register embedded Fira Code font faces");
    }
}

/// Primary UI font family (`Noto Sans SC`).
pub const fn ui_font_family() -> &'static str {
    "Noto Sans SC"
}

/// Monospace font family (`Fira Code`, embedded cross-platform).
pub const fn mono_font_family() -> &'static str {
    "Fira Code"
}

/// Calibrated bold weight for `Noto Sans SC` UI headings and labels.
/// On Windows DirectWrite, `NotoSansSC-VF.ttf` applies heavy synthetic bold at weight 700,
/// so `SEMIBOLD` (600) produces balanced stroke thickness.
pub const fn ui_weight_bold() -> FontWeight {
    #[cfg(target_os = "windows")]
    {
        FontWeight::SEMIBOLD
    }
    #[cfg(not(target_os = "windows"))]
    {
        FontWeight::BOLD
    }
}

/// Resolved GPUI color palette derived strictly from `clipboardx_model::ThemeTokens`.
#[derive(Clone, Debug)]
pub struct ThemePalette {
    pub is_dark: bool,
    pub bg_window: Hsla,
    pub bg_surface: Hsla,
    pub bg_elevated: Hsla,
    pub bg_hover: Hsla,
    pub bg_selected: Hsla,
    pub bg_code: Hsla,
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub text_muted: Hsla,
    pub accent_blue: Hsla,
    pub accent_blue_soft: Hsla,
    pub accent_green: Hsla,
    pub accent_green_soft: Hsla,
    pub accent_amber: Hsla,
    pub accent_amber_soft: Hsla,
    pub accent_purple: Hsla,
    pub accent_purple_soft: Hsla,
    pub accent_red: Hsla,
    pub shadow_primary_alpha: f32,
    pub shadow_tight_alpha: f32,
}

impl ThemePalette {
    /// Builds a GPUI palette from the SSOT theme identifier.
    pub fn from_theme_id(theme_id: ThemeId) -> Self {
        let tokens: ThemeTokens = theme_id.tokens();
        Self {
            is_dark: theme_id.is_dark(),
            bg_window: hex_to_hsla(tokens.bg_window, 1.0),
            bg_surface: hex_to_hsla(tokens.bg_surface, 1.0),
            bg_elevated: hex_to_hsla(tokens.bg_elevated, 1.0),
            bg_hover: hex_to_hsla(tokens.bg_hover, 1.0),
            bg_selected: hex_to_hsla(tokens.bg_selected, 1.0),
            bg_code: hex_to_hsla(tokens.bg_code, 1.0),
            text_primary: hex_to_hsla(tokens.text_primary, 1.0),
            text_secondary: hex_to_hsla(tokens.text_secondary, 1.0),
            text_muted: hex_to_hsla(tokens.text_muted, 1.0),
            accent_blue: hex_to_hsla(tokens.accent_blue, 1.0),
            accent_blue_soft: hex_to_hsla(tokens.accent_blue_soft, 1.0),
            accent_green: hex_to_hsla(tokens.accent_green, 1.0),
            accent_green_soft: hex_to_hsla(tokens.accent_green_soft, 1.0),
            accent_amber: hex_to_hsla(tokens.accent_amber, 1.0),
            accent_amber_soft: hex_to_hsla(tokens.accent_amber_soft, 1.0),
            accent_purple: hex_to_hsla(tokens.accent_purple, 1.0),
            accent_purple_soft: hex_to_hsla(tokens.accent_purple_soft, 1.0),
            accent_red: hex_to_hsla(tokens.accent_red, 1.0),
            shadow_primary_alpha: f32::from(tokens.shadow_primary_alpha) / 255.0,
            shadow_tight_alpha: f32::from(tokens.shadow_tight_alpha) / 255.0,
        }
    }

    /// Dual-layer anti-aliased custom drop shadow sized cleanly within the `22px` outer
    /// transparent shadow margin so arbitrary corner radii (including `r=999` oval capsules)
    /// cast smooth, unclipped shadows with zero OS DWM square artifacts.
    pub fn window_shadows(&self) -> Vec<BoxShadow> {
        vec![
            BoxShadow {
                color: rgba_black(self.shadow_primary_alpha * 0.72),
                offset: point(px(0.0), px(7.0)),
                blur_radius: px(18.0),
                spread_radius: px(-3.0),
                inset: false,
            },
            BoxShadow {
                color: rgba_black(self.shadow_tight_alpha * 0.85),
                offset: point(px(0.0), px(2.0)),
                blur_radius: px(6.0),
                spread_radius: px(0.0),
                inset: false,
            },
        ]
    }

    /// Smoothly interpolates every semantic color token between two themes for the
    /// 280ms dark/light hot theme cross-fade.
    pub fn lerp(from: &Self, to: &Self, t: f32) -> Self {
        let p = t.clamp(0.0, 1.0);
        if p <= 0.001 {
            return from.clone();
        }
        if p >= 0.999 {
            return to.clone();
        }
        Self {
            is_dark: if p < 0.5 { from.is_dark } else { to.is_dark },
            bg_window: lerp_hsla(from.bg_window, to.bg_window, p),
            bg_surface: lerp_hsla(from.bg_surface, to.bg_surface, p),
            bg_elevated: lerp_hsla(from.bg_elevated, to.bg_elevated, p),
            bg_hover: lerp_hsla(from.bg_hover, to.bg_hover, p),
            bg_selected: lerp_hsla(from.bg_selected, to.bg_selected, p),
            bg_code: lerp_hsla(from.bg_code, to.bg_code, p),
            text_primary: lerp_hsla(from.text_primary, to.text_primary, p),
            text_secondary: lerp_hsla(from.text_secondary, to.text_secondary, p),
            text_muted: lerp_hsla(from.text_muted, to.text_muted, p),
            accent_blue: lerp_hsla(from.accent_blue, to.accent_blue, p),
            accent_blue_soft: lerp_hsla(from.accent_blue_soft, to.accent_blue_soft, p),
            accent_green: lerp_hsla(from.accent_green, to.accent_green, p),
            accent_green_soft: lerp_hsla(from.accent_green_soft, to.accent_green_soft, p),
            accent_amber: lerp_hsla(from.accent_amber, to.accent_amber, p),
            accent_amber_soft: lerp_hsla(from.accent_amber_soft, to.accent_amber_soft, p),
            accent_purple: lerp_hsla(from.accent_purple, to.accent_purple, p),
            accent_purple_soft: lerp_hsla(from.accent_purple_soft, to.accent_purple_soft, p),
            accent_red: lerp_hsla(from.accent_red, to.accent_red, p),
            shadow_primary_alpha: lerp_f32(from.shadow_primary_alpha, to.shadow_primary_alpha, p),
            shadow_tight_alpha: lerp_f32(from.shadow_tight_alpha, to.shadow_tight_alpha, p),
        }
    }
}

/// Motion timing and offset constants (`cubic-bezier(0.22, 1.0, 0.36, 1.0)` shared across transitions).
pub struct MotionSpec;

impl MotionSpec {
    /// Dual-pane collapse (`680 -> 328`) shell width duration.
    pub const DRAWER_COLLAPSE_MS: f32 = 180.0;

    /// Slim expand (`328 -> 680`) shell width duration.
    pub const DRAWER_EXPAND_MS: f32 = 180.0;

    /// Capsule hover spring anticipation and hover open threshold.
    pub const CAPSULE_HOVER_SPRING_MS: f32 = 180.0;
    pub const CAPSULE_HOVER_OPEN_MS: u64 = 20;
    pub const CAPSULE_HOVER_ICON_SCALE: f32 = 1.14;
    /// Panel to mini-capsule collapse duration.
    pub const CAPSULE_COLLAPSE_MS: f32 = 220.0;
    /// Two-phase landing bounce (`90ms: 1.0->0.94`, `120ms: 0.94->1.0`).
    pub const CAPSULE_SETTLE_DOWN_MS: f32 = 90.0;
    pub const CAPSULE_SETTLE_UP_MS: f32 = 120.0;
    pub const CAPSULE_SETTLE_TOTAL_MS: f32 =
        Self::CAPSULE_SETTLE_DOWN_MS + Self::CAPSULE_SETTLE_UP_MS;
    pub const CAPSULE_SETTLE_MIN_SCALE: f32 = 0.94;

    /// Mini-capsule to main panel expansion duration.
    pub const FULL_EXPAND_MS: f32 = 220.0;

    /// Global dark/light theme cross-fade duration.
    pub const THEME_CROSSFADE_MS: f32 = 280.0;

    /// Settings view enter and exit transition durations and offsets.
    pub const SETTINGS_ENTER_MS: f32 = 220.0;
    pub const SETTINGS_ENTER_SLIDE_X: f32 = 16.0;
    pub const SETTINGS_EXIT_MS: f32 = 220.0;
    pub const SETTINGS_EXIT_SLIDE_X: f32 = -12.0;

    /// Image hover preview card entrance duration and horizontal slide offset.
    /// `8px` slide distance matches `HOVER_CARD_GAP` so frame 0 starts flush outside the panel edge
    /// without overlapping the main card.
    pub const IMAGE_HOVER_ENTER_MS: f32 = 220.0;
    pub const IMAGE_HOVER_SLIDE_X: f32 = 8.0;

    /// Tray context menu pop-in duration (`160ms`, `+6px -> 0`).
    pub const TRAY_MENU_ENTER_MS: f32 = 160.0;
    pub const TRAY_MENU_SLIDE_Y: f32 = 6.0;
}

/// Linearly interpolates between `a` and `b` at normalized progress `t`.
pub fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Evaluates `cubic-bezier(0.22, 1.0, 0.36, 1.0)` shared across window transitions.
pub fn ease_out_expo_cubic(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let mut u = t;
    for _ in 0..6 {
        let one_m = 1.0 - u;
        let bx = 3.0 * one_m * one_m * u * 0.22 + 3.0 * one_m * u * u * 0.36 + u * u * u;
        let dbx = 3.0 * one_m * one_m * 0.22
            + 6.0 * one_m * u * (0.36 - 0.22)
            + 3.0 * u * u * (1.0 - 0.36);
        if dbx.abs() < 1e-6 {
            break;
        }
        u = (u - (bx - t) / dbx).clamp(0.0, 1.0);
    }
    let one_m = 1.0 - u;
    (1.0 - one_m * one_m * one_m).clamp(0.0, 1.0)
}

/// Underdamped spring curve for capsule hover anticipation (`peak ~1.108`).
pub fn spring_overshoot(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let decay = (-6.2 * t).exp();
    1.0 - decay * (9.2 * t).cos()
}

/// Linearly interpolates two GPUI HSLA colors in linear RGBA space.
pub fn lerp_hsla(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let ra: Rgba = a.into();
    let rb: Rgba = b.into();
    Rgba {
        r: lerp_f32(ra.r, rb.r, t),
        g: lerp_f32(ra.g, rb.g, t),
        b: lerp_f32(ra.b, rb.b, t),
        a: lerp_f32(ra.a, rb.a, t),
    }
    .into()
}

/// Resolves the effective theme identifier from user preference and OS appearance.
pub fn resolve_theme_id(config: &AppConfig, appearance: WindowAppearance) -> ThemeId {
    let os_is_dark = matches!(
        appearance,
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    );
    match config.theme_mode {
        ThemeMode::System => {
            if os_is_dark {
                config.dark_theme
            } else {
                config.light_theme
            }
        }
        ThemeMode::AlwaysDark => config.dark_theme,
        ThemeMode::AlwaysLight => config.light_theme,
    }
}

/// Synchronizes `gpui-component` global theme tokens so embedded controls
/// (`Input`) render seamlessly inside borderless tonal surfaces.
pub fn sync_component_theme(palette: &ThemePalette, window: Option<&mut Window>, cx: &mut App) {
    let mode = if palette.is_dark {
        ComponentThemeMode::Dark
    } else {
        ComponentThemeMode::Light
    };
    ComponentTheme::change(mode, window, cx);
    let theme = ComponentTheme::global_mut(cx);
    theme.font_family = ui_font_family().into();
    theme.mono_font_family = mono_font_family().into();
    // Keep the outer GPUI Root surface fully transparent so custom rounded shadows render
    // cleanly inside the transparent shadow margin without an opaque rectangular backing.
    theme.background = gpui_kit::transparent_black();
    theme.tokens.background = gpui_kit::transparent_black().into();
    theme.foreground = palette.text_primary;
    theme.muted_foreground = palette.text_muted;
    theme.caret = palette.accent_blue;
    theme.selection = palette.accent_blue_soft;
}

/// Converts a 24-bit `RgbHex` plus alpha into a GPUI `Hsla` color.
pub fn hex_to_hsla(hex: RgbHex, alpha: f32) -> Hsla {
    let [r, g, b, a] = hex.as_rgba_f32(alpha);
    Rgba { r, g, b, a }.into()
}

/// Parses `#RRGGBB` string colors for dynamic swatch tiles.
pub fn parse_hex_str_hsla(raw: &str) -> Option<Hsla> {
    let trimmed = raw.trim().strip_prefix('#')?;
    if trimmed.len() != 6 {
        return None;
    }
    let val = u32::from_str_radix(trimmed, 16).ok()?;
    Some(hex_to_hsla(RgbHex(val), 1.0))
}

/// Extracts a color swatch `Hsla` from `preview_text` or `meta_json` without allocating a JSON DOM.
pub fn extract_color_swatch_hsla(preview_text: &str, meta_json: &str) -> Option<Hsla> {
    if let Some(c) = parse_hex_str_hsla(preview_text) {
        return Some(c);
    }
    let idx = meta_json.find("\"color_hex\"")?;
    let rest = &meta_json[idx + 11..];
    let hash_idx = rest.find('#')?;
    let hex_slice = rest.get(hash_idx..hash_idx + 7)?;
    parse_hex_str_hsla(hex_slice)
}

/// Picks crisp readable foreground text (dark or white) for a color swatch background.
pub fn contrasting_fg_for_hex(raw: &str) -> Hsla {
    let Some(trimmed) = raw.trim().strip_prefix('#') else {
        return hex_to_hsla(RgbHex(0xFFFFFF), 1.0);
    };
    let Ok(val) = u32::from_str_radix(&trimmed[..trimmed.len().min(6)], 16) else {
        return hex_to_hsla(RgbHex(0xFFFFFF), 1.0);
    };
    let hex = RgbHex(val);
    let lum = 0.2126 * (f32::from(hex.r_u8()) / 255.0)
        + 0.7152 * (f32::from(hex.g_u8()) / 255.0)
        + 0.0722 * (f32::from(hex.b_u8()) / 255.0);
    if lum > 0.56 {
        hex_to_hsla(RgbHex(0x0F172A), 1.0)
    } else {
        hex_to_hsla(RgbHex(0xF8FAFC), 1.0)
    }
}

fn rgba_black(alpha: f32) -> Hsla {
    Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: alpha,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_out_expo_cubic_matches_standard_bezier_curve() {
        assert_eq!(ease_out_expo_cubic(-0.5), 0.0);
        assert_eq!(ease_out_expo_cubic(0.0), 0.0);
        assert_eq!(ease_out_expo_cubic(1.0), 1.0);
        assert_eq!(ease_out_expo_cubic(1.5), 1.0);

        // Monotonic deceleration curve matching CSS `cubic-bezier(0.22, 1.0, 0.36, 1.0)`.
        let mut prev = 0.0_f32;
        for step in 1..=20 {
            let t = step as f32 / 20.0;
            let y = ease_out_expo_cubic(t);
            assert!(y >= prev, "curve must be monotonic at t={t}: {y} < {prev}");
            assert!(y <= 1.0);
            prev = y;
        }
        // Fast initial acceleration, gentle tail settle (at t=0.25, y is already > 0.72).
        let y25 = ease_out_expo_cubic(0.25);
        let y50 = ease_out_expo_cubic(0.50);
        assert!(y25 > 0.72 && y25 < 0.85, "unexpected y(0.25) = {y25}");
        assert!(y50 > 0.90 && y50 < 0.98, "unexpected y(0.50) = {y50}");
    }

    #[test]
    fn spring_overshoot_peaks_above_one_and_settles_at_one() {
        assert_eq!(spring_overshoot(0.0), 0.0);
        assert_eq!(spring_overshoot(1.0), 1.0);
        let peak = (1..100)
            .map(|i| spring_overshoot(i as f32 / 100.0))
            .fold(0.0_f32, f32::max);
        assert!(peak > 1.05 && peak < 1.16, "unexpected spring peak {peak}");
    }

    #[test]
    fn motion_spec_timings_match_expected_durations() {
        assert_eq!(MotionSpec::DRAWER_COLLAPSE_MS, 180.0);
        assert_eq!(MotionSpec::DRAWER_EXPAND_MS, 180.0);
        assert_eq!(MotionSpec::FULL_EXPAND_MS, 220.0);
        assert_eq!(MotionSpec::CAPSULE_COLLAPSE_MS, 220.0);
        assert_eq!(MotionSpec::CAPSULE_SETTLE_TOTAL_MS, 210.0);
        assert_eq!(MotionSpec::THEME_CROSSFADE_MS, 280.0);
        assert_eq!(MotionSpec::IMAGE_HOVER_ENTER_MS, 220.0);
        assert_eq!(MotionSpec::IMAGE_HOVER_SLIDE_X, 8.0);
    }

    #[test]
    fn extract_color_swatch_hsla_parses_direct_hex_and_meta_json_without_alloc() {
        let direct = extract_color_swatch_hsla("#3B82F6", "{}");
        let from_meta =
            extract_color_swatch_hsla("rgb(59, 130, 246)", "{\"color_hex\":\"#3B82F6\"}");
        assert!(direct.is_some());
        assert_eq!(direct, from_meta);
        assert!(extract_color_swatch_hsla("not a color", "{}").is_none());
    }
}
