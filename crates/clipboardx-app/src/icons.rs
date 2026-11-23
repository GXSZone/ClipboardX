use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};

use clipboardx_platform::{
    begin_native_window_drag, cursor_screen_pos, move_own_window_visual_to,
    own_window_visual_origin,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AssetSource, ElementId, Hsla, IntoElement, MouseButton, MouseMoveEvent, Pixels,
    Point, Result, Rgba, SharedString, Styled, Window, div, px, svg,
};

static BLIND_DRAG_ACTIVE: AtomicBool = AtomicBool::new(false);
static BLIND_DRAG_CONSUMED: AtomicBool = AtomicBool::new(false);
static BLIND_DRAG_SHIELDED: AtomicBool = AtomicBool::new(false);

/// Built-in vector icons embedded into the application binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIcon {
    Search,
    CollapseSidebar,
    ExpandSidebar,
    Pause,
    Settings,
    Close,
    ArrowLeft,
    Shield,
    Priv,
    Plus,
    Key,
    Bolt,
    Eye,
    Download,
    Upload,
    CheckUpdate,
    NewVer,
    External,
    Code,
    Link,
    Text,
    File,
    Copy,
    Clipboard,
    BrandMarkFill,
    BrandMarkFrame,
    BrandMarkXMain,
    BrandMarkXAccent,
    BrandAppCard,
    BrandAppClip,
}

impl AppIcon {
    pub const fn asset_path(self) -> &'static str {
        match self {
            Self::Search => "icons/search.svg",
            Self::CollapseSidebar => "icons/collapse_sidebar.svg",
            Self::ExpandSidebar => "icons/expand_sidebar.svg",
            Self::Pause => "icons/pause.svg",
            Self::Settings => "icons/settings.svg",
            Self::Close => "icons/close.svg",
            Self::ArrowLeft => "icons/arrow_left.svg",
            Self::Shield => "icons/shield.svg",
            Self::Priv => "icons/priv.svg",
            Self::Plus => "icons/plus.svg",
            Self::Key => "icons/key.svg",
            Self::Bolt => "icons/bolt.svg",
            Self::Eye => "icons/eye.svg",
            Self::Download => "icons/download.svg",
            Self::Upload => "icons/upload.svg",
            Self::CheckUpdate => "icons/check_update.svg",
            Self::NewVer => "icons/new_ver.svg",
            Self::External => "icons/external.svg",
            Self::Code => "icons/code.svg",
            Self::Link => "icons/link.svg",
            Self::Text => "icons/text.svg",
            Self::File => "icons/file.svg",
            Self::Copy => "icons/copy.svg",
            Self::Clipboard => "icons/clipboard.svg",
            Self::BrandMarkFill => "icons/brand_mark_fill.svg",
            Self::BrandMarkFrame => "icons/brand_mark_frame.svg",
            Self::BrandMarkXMain => "icons/brand_mark_x_main.svg",
            Self::BrandMarkXAccent => "icons/brand_mark_x_accent.svg",
            Self::BrandAppCard => "icons/brand_app_card.svg",
            Self::BrandAppClip => "icons/brand_app_clip.svg",
        }
    }
}

const APP_SVG_ASSETS: &[(&str, &[u8])] = &[
    (
        AppIcon::Search.asset_path(),
        include_bytes!("../../../assets/icons/search.svg"),
    ),
    (
        AppIcon::CollapseSidebar.asset_path(),
        include_bytes!("../../../assets/icons/collapse_sidebar.svg"),
    ),
    (
        AppIcon::ExpandSidebar.asset_path(),
        include_bytes!("../../../assets/icons/expand_sidebar.svg"),
    ),
    (
        AppIcon::Pause.asset_path(),
        include_bytes!("../../../assets/icons/pause.svg"),
    ),
    (
        AppIcon::Settings.asset_path(),
        include_bytes!("../../../assets/icons/settings.svg"),
    ),
    (
        AppIcon::Close.asset_path(),
        include_bytes!("../../../assets/icons/close.svg"),
    ),
    (
        AppIcon::ArrowLeft.asset_path(),
        include_bytes!("../../../assets/icons/arrow_left.svg"),
    ),
    (
        AppIcon::Shield.asset_path(),
        include_bytes!("../../../assets/icons/shield.svg"),
    ),
    (
        AppIcon::Priv.asset_path(),
        include_bytes!("../../../assets/icons/priv.svg"),
    ),
    (
        AppIcon::Plus.asset_path(),
        include_bytes!("../../../assets/icons/plus.svg"),
    ),
    (
        AppIcon::Key.asset_path(),
        include_bytes!("../../../assets/icons/key.svg"),
    ),
    (
        AppIcon::Bolt.asset_path(),
        include_bytes!("../../../assets/icons/bolt.svg"),
    ),
    (
        AppIcon::Eye.asset_path(),
        include_bytes!("../../../assets/icons/eye.svg"),
    ),
    (
        AppIcon::Download.asset_path(),
        include_bytes!("../../../assets/icons/download.svg"),
    ),
    (
        AppIcon::Upload.asset_path(),
        include_bytes!("../../../assets/icons/upload.svg"),
    ),
    (
        AppIcon::CheckUpdate.asset_path(),
        include_bytes!("../../../assets/icons/check_update.svg"),
    ),
    (
        AppIcon::NewVer.asset_path(),
        include_bytes!("../../../assets/icons/new_ver.svg"),
    ),
    (
        AppIcon::External.asset_path(),
        include_bytes!("../../../assets/icons/external.svg"),
    ),
    (
        AppIcon::Code.asset_path(),
        include_bytes!("../../../assets/icons/code.svg"),
    ),
    (
        AppIcon::Link.asset_path(),
        include_bytes!("../../../assets/icons/link.svg"),
    ),
    (
        AppIcon::Text.asset_path(),
        include_bytes!("../../../assets/icons/text.svg"),
    ),
    (
        AppIcon::File.asset_path(),
        include_bytes!("../../../assets/icons/file.svg"),
    ),
    (
        AppIcon::Copy.asset_path(),
        include_bytes!("../../../assets/icons/copy.svg"),
    ),
    (
        AppIcon::Clipboard.asset_path(),
        include_bytes!("../../../assets/icons/clipboard.svg"),
    ),
    (
        AppIcon::BrandMarkFill.asset_path(),
        include_bytes!("../../../assets/icons/brand_mark_fill.svg"),
    ),
    (
        AppIcon::BrandMarkFrame.asset_path(),
        include_bytes!("../../../assets/icons/brand_mark_frame.svg"),
    ),
    (
        AppIcon::BrandMarkXMain.asset_path(),
        include_bytes!("../../../assets/icons/brand_mark_x_main.svg"),
    ),
    (
        AppIcon::BrandMarkXAccent.asset_path(),
        include_bytes!("../../../assets/icons/brand_mark_x_accent.svg"),
    ),
    (
        AppIcon::BrandAppCard.asset_path(),
        include_bytes!("../../../assets/icons/brand_app_card.svg"),
    ),
    (
        AppIcon::BrandAppClip.asset_path(),
        include_bytes!("../../../assets/icons/brand_app_clip.svg"),
    ),
];

/// Unified asset source serving bundled application SVG icons first,
/// falling back to `gpui_kit::assets::AllAssets` for supplementary controls.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = APP_SVG_ASSETS.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items = gpui_kit::assets::AllAssets.list(path)?;
        for (p, _) in APP_SVG_ASSETS {
            if p.starts_with(path) {
                items.push((*p).into());
            }
        }
        Ok(items)
    }
}

/// Renders a bundled vector SVG icon tinted with `color`.
pub fn app_icon(icon: AppIcon, size_px: f32, color: Hsla) -> AnyElement {
    svg()
        .path(icon.asset_path())
        .size(px(size_px))
        .flex_shrink_0()
        .text_color(color)
        .into_any_element()
}

fn rgba_hex(hex: u32, alpha: f32) -> Hsla {
    Rgba {
        r: ((hex >> 16) & 0xFF) as f32 / 255.0,
        g: ((hex >> 8) & 0xFF) as f32 / 255.0,
        b: (hex & 0xFF) as f32 / 255.0,
        a: alpha,
    }
    .into()
}

/// Renders the layered dual-tone ClipboardX brand mark used inside the floating glass capsule.
pub fn brand_capsule_mark(size_px: f32, is_dark: bool) -> AnyElement {
    let (body_fill, frame_color, x_main, x_accent) = if is_dark {
        (
            rgba_hex(0x60A5FA, 0.24),
            rgba_hex(0x60A5FA, 1.0),
            rgba_hex(0xF8FAFC, 1.0),
            rgba_hex(0x60A5FA, 1.0),
        )
    } else {
        (
            rgba_hex(0x3B82F6, 0.14),
            rgba_hex(0x2563EB, 1.0),
            rgba_hex(0x1E3A8A, 1.0),
            rgba_hex(0x2563EB, 1.0),
        )
    };

    let layer = |icon: AppIcon, color: Hsla| {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(app_icon(icon, size_px, color))
    };

    div()
        .relative()
        .size(px(size_px))
        .flex_shrink_0()
        .child(layer(AppIcon::BrandMarkFill, body_fill))
        .child(layer(AppIcon::BrandMarkFrame, frame_color))
        .child(layer(AppIcon::BrandMarkXMain, x_main))
        .child(layer(AppIcon::BrandMarkXAccent, x_accent))
        .into_any_element()
}

/// Renders the ClipboardX squircle application icon with layered clipboard card and dual-tone X.
pub fn brand_app_icon(size_px: f32) -> AnyElement {
    let radius = (size_px * 0.225).max(5.0);
    let layer = |icon: AppIcon, color: Hsla| {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(app_icon(icon, size_px, color))
    };

    div()
        .relative()
        .size(px(size_px))
        .rounded(px(radius))
        .overflow_hidden()
        .flex_shrink_0()
        .bg(rgba_hex(0x172554, 1.0))
        .child(
            div()
                .absolute()
                .right(px(-size_px * 0.18))
                .bottom(px(-size_px * 0.22))
                .size(px(size_px * 0.88))
                .rounded(px(999.0))
                .bg(rgba_hex(0x2563EB, 0.62)),
        )
        .child(layer(AppIcon::BrandAppCard, rgba_hex(0xF8FAFC, 0.98)))
        .child(layer(AppIcon::BrandAppClip, rgba_hex(0x3B82F6, 1.0)))
        .child(layer(AppIcon::BrandMarkXMain, rgba_hex(0x1D4ED8, 1.0)))
        .child(layer(AppIcon::BrandMarkXAccent, rgba_hex(0x38BDF8, 1.0)))
        .into_any_element()
}

/// Renders a supplementary built-in kit icon for auxiliary controls.
pub fn kit_icon(name: IconName, size_px: f32, color: Hsla) -> AnyElement {
    Icon::new(name)
        .with_size(px(size_px))
        .text_color(color)
        .into_any_element()
}

/// Wraps an interactive text input container so mouse-drag text selection never triggers window dragging.
pub fn input_shield(id: impl Into<ElementId>, inner: AnyElement) -> AnyElement {
    div()
        .id(id)
        .w_full()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            BLIND_DRAG_SHIELDED.store(true, Ordering::Release);
            BLIND_DRAG_ACTIVE.store(false, Ordering::Release);
            BLIND_DRAG_CONSUMED.store(false, Ordering::Release);
            cx.stop_propagation();
        })
        .child(inner)
        .into_any_element()
}

/// Returns whether a blind window drag is currently in progress.
pub fn is_blind_dragging() -> bool {
    BLIND_DRAG_ACTIVE.load(Ordering::Acquire)
}

/// Returns `true` if the latest pointer press-move-release cycle was consumed by a blind
/// window drag (`>= 4px` movement), preventing accidental item activation or button clicks.
pub fn was_blind_drag_consumed() -> bool {
    BLIND_DRAG_CONSUMED.load(Ordering::Acquire)
}

/// Resets blind-drag consumption flags on a fresh left-button press inside an interactive row or pill.
pub fn reset_blind_drag_click_guard() {
    BLIND_DRAG_SHIELDED.store(false, Ordering::Release);
    BLIND_DRAG_CONSUMED.store(false, Ordering::Release);
}

/// Tracks pointer press + 4px threshold across the entire borderless window (including list items,
/// preview cards, quick-paste cards, and the mini-capsule) so users can blind-drag anywhere
/// outside text inputs while preserving crisp `< 4px` click-release activation.
#[derive(Default)]
pub struct BlindDragTracker {
    press_window_point: Option<Point<Pixels>>,
    press_screen_cursor: Option<(i32, i32)>,
    press_visual_origin: Option<(i32, i32)>,
    dragging: bool,
}

impl BlindDragTracker {
    pub fn on_mouse_down(&mut self, position: Point<Pixels>) {
        // Any mouse-down reaching the outer shell was not intercepted by `input_shield`'s
        // `stop_propagation()`, so we always clear stale shield flags and arm a fresh press.
        BLIND_DRAG_SHIELDED.store(false, Ordering::Release);
        self.press_window_point = Some(position);
        self.press_screen_cursor = cursor_screen_pos();
        self.press_visual_origin = own_window_visual_origin();
        self.dragging = false;
        BLIND_DRAG_ACTIVE.store(false, Ordering::Release);
        BLIND_DRAG_CONSUMED.store(false, Ordering::Release);
    }

    /// Finishes the current pointer press/drag cycle and returns `true` if the window was moved.
    pub fn on_mouse_up(&mut self) -> bool {
        let was_dragging = self.dragging || BLIND_DRAG_ACTIVE.load(Ordering::Acquire);
        self.clear();
        was_dragging
    }

    fn clear(&mut self) {
        self.press_window_point = None;
        self.press_screen_cursor = None;
        self.press_visual_origin = None;
        self.dragging = false;
        BLIND_DRAG_ACTIVE.store(false, Ordering::Release);
        BLIND_DRAG_SHIELDED.store(false, Ordering::Release);
    }

    pub fn on_mouse_move(&mut self, ev: &MouseMoveEvent, window: &mut Window) -> bool {
        if ev.pressed_button != Some(MouseButton::Left) {
            self.clear();
            return false;
        }
        if BLIND_DRAG_SHIELDED.load(Ordering::Acquire) {
            return false;
        }
        if self.press_window_point.is_none() {
            self.press_window_point = Some(ev.position);
            self.press_screen_cursor = cursor_screen_pos();
            self.press_visual_origin = own_window_visual_origin();
            self.dragging = false;
            BLIND_DRAG_CONSUMED.store(false, Ordering::Release);
        }

        if let (Some((start_cx, start_cy)), Some((start_wx, start_wy)), Some((cur_cx, cur_cy))) = (
            self.press_screen_cursor,
            self.press_visual_origin,
            cursor_screen_pos(),
        ) {
            let dx = cur_cx - start_cx;
            let dy = cur_cy - start_cy;
            if !self.dragging && (dx * dx + dy * dy) >= 16 {
                self.dragging = true;
                BLIND_DRAG_ACTIVE.store(true, Ordering::Release);
                BLIND_DRAG_CONSUMED.store(true, Ordering::Release);
            }
            if self.dragging {
                let _ = move_own_window_visual_to(start_wx + dx, start_wy + dy);
                return true;
            }
            return false;
        }

        let Some(origin) = self.press_window_point else {
            return false;
        };
        let dx = f32::from(ev.position.x - origin.x);
        let dy = f32::from(ev.position.y - origin.y);
        if dx * dx + dy * dy >= 16.0 {
            self.clear();
            BLIND_DRAG_CONSUMED.store(true, Ordering::Release);
            begin_native_window_drag();
            window.start_window_move();
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::point;

    #[test]
    fn blind_drag_click_guard_resets_and_preserves_consumption_until_next_press() {
        reset_blind_drag_click_guard();
        assert!(!is_blind_dragging());
        assert!(!was_blind_drag_consumed());

        BLIND_DRAG_ACTIVE.store(true, Ordering::Release);
        BLIND_DRAG_CONSUMED.store(true, Ordering::Release);
        assert!(is_blind_dragging());
        assert!(was_blind_drag_consumed());

        let mut tracker = BlindDragTracker {
            dragging: true,
            ..Default::default()
        };
        assert!(tracker.on_mouse_up());
        assert!(!is_blind_dragging());
        // Consumed flag stays true through all mouse-up / click listeners of the current release
        assert!(was_blind_drag_consumed());

        // Next fresh mouse press clears the consumed flag
        tracker.on_mouse_down(point(px(12.0), px(18.0)));
        assert!(!was_blind_drag_consumed());

        // Even if a prior input_shield press left BLIND_DRAG_SHIELDED = true (e.g. released outside
        // the window), a fresh non-input press on the shell clears it and arms blind dragging.
        BLIND_DRAG_SHIELDED.store(true, Ordering::Release);
        tracker.on_mouse_down(point(px(40.0), px(60.0)));
        assert!(!BLIND_DRAG_SHIELDED.load(Ordering::Acquire));
        assert!(tracker.press_window_point.is_some());
    }
}
