use std::rc::Rc;
use std::time::Instant;

use clipboardx_platform::TrayAction;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, FocusHandle, Focusable, FontWeight, KeyDownEvent, MouseButton,
    Render, Window, div, px,
};
use rust_i18n::t;

use crate::icons::{AppIcon, app_icon, brand_app_icon};
use crate::shell::WINDOW_SHADOW_MARGIN;
use crate::state::AppRuntime;
use crate::theme::{
    MotionSpec, ThemePalette, ease_out_expo_cubic, lerp_f32, mono_font_family, resolve_theme_id,
    ui_font_family, ui_weight_bold,
};

/// Visual logical dimensions of the minimalist borderless GPUI tray context menu.
pub const TRAY_MENU_WIDTH: f32 = 200.0;
pub const TRAY_MENU_HEIGHT: f32 = 178.0;

/// Minimalist 0px-border GPUI tray context menu matching ClipboardX's tonal rounded aesthetic.
pub struct TrayMenuView {
    runtime: Rc<AppRuntime>,
    on_action: Rc<dyn Fn(TrayAction)>,
    focus_handle: FocusHandle,
    entrance_started_at: Option<Instant>,
}

impl Focusable for TrayMenuView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TrayMenuView {
    pub fn new(
        runtime: Rc<AppRuntime>,
        on_action: Rc<dyn Fn(TrayAction)>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        Self {
            runtime,
            on_action,
            focus_handle,
            entrance_started_at: Some(Instant::now()),
        }
    }

    fn handle_key_down(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if ev.keystroke.key.as_str() == "escape" {
            window.remove_window();
            cx.stop_propagation();
        }
    }

    fn trigger(&self, action: TrayAction, window: &mut Window) {
        (self.on_action)(action);
        window.remove_window();
    }
}

impl Render for TrayMenuView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let cfg = self.runtime.snapshot_config();
        let theme_id = resolve_theme_id(&cfg, window.appearance());
        let palette = ThemePalette::from_theme_id(theme_id);
        let is_paused = self.runtime.is_paused();

        let p = if let Some(started_at) = self.entrance_started_at {
            let raw = (started_at.elapsed().as_secs_f32() * 1000.0
                / MotionSpec::TRAY_MENU_ENTER_MS)
                .clamp(0.0, 1.0);
            if raw < 1.0 {
                window.request_animation_frame();
                ease_out_expo_cubic(raw)
            } else {
                self.entrance_started_at = None;
                1.0
            }
        } else {
            1.0
        };
        let offset_y = lerp_f32(MotionSpec::TRAY_MENU_SLIDE_Y, 0.0, p);

        let pause_label = if is_paused {
            t!("tray.resume").to_string()
        } else {
            t!("tray.pause_manual").to_string()
        };
        let status_dot = if is_paused {
            palette.accent_amber
        } else {
            palette.accent_green
        };
        let status_text = if is_paused {
            t!("footer.paused").to_string()
        } else {
            t!("footer.encrypted_local").to_string()
        };

        div()
            .id("tray-menu-shadow-host")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .size_full()
            .p(px(WINDOW_SHADOW_MARGIN))
            .bg(gpui_kit::transparent_black())
            .child(
                div()
                    .id("tray-menu-card")
                    .relative()
                    .top(px(offset_y))
                    .size_full()
                    .opacity(p)
                    .font_family(ui_font_family())
                    .rounded(px(12.0))
                    .bg(palette.bg_surface)
                    .shadow(palette.window_shadows())
                    .p(px(6.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    // Minimalist brand + live status header (0px borders)
                    .child(
                        div()
                            .w_full()
                            .px(px(8.0))
                            .pt(px(4.0))
                            .pb(px(6.0))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .child(brand_app_icon(18.0))
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .line_height(px(14.0))
                                            .font_weight(ui_weight_bold())
                                            .text_color(palette.text_primary)
                                            .child("ClipboardX"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .child(div().size(px(5.5)).rounded(px(999.0)).bg(status_dot))
                                    .child(
                                        div()
                                            .text_size(px(9.5))
                                            .line_height(px(12.0))
                                            .text_color(palette.text_muted)
                                            .child(status_text),
                                    ),
                            ),
                    )
                    .child(tray_menu_row(
                        TrayRowSpec {
                            id: "tray-item-show",
                            icon: AppIcon::Clipboard,
                            label: &t!("tray.show_hide"),
                            shortcut: Some(cfg.toggle_hotkey.as_str()),
                            icon_color: palette.accent_blue,
                            label_color: palette.text_primary,
                        },
                        &palette,
                        cx.listener(|this, _, window, _| {
                            this.trigger(TrayAction::ShowOrHide, window);
                        }),
                    ))
                    .child(tray_menu_row(
                        TrayRowSpec {
                            id: "tray-item-pause",
                            icon: AppIcon::Pause,
                            label: &pause_label,
                            shortcut: Some(cfg.pause_hotkey.as_str()),
                            icon_color: if is_paused {
                                palette.accent_amber
                            } else {
                                palette.text_secondary
                            },
                            label_color: if is_paused {
                                palette.accent_amber
                            } else {
                                palette.text_primary
                            },
                        },
                        &palette,
                        cx.listener(|this, _, window, _| {
                            this.trigger(TrayAction::TogglePause, window);
                        }),
                    ))
                    .child(tray_menu_row(
                        TrayRowSpec {
                            id: "tray-item-settings",
                            icon: AppIcon::Settings,
                            label: &t!("tray.settings"),
                            shortcut: None,
                            icon_color: palette.text_secondary,
                            label_color: palette.text_primary,
                        },
                        &palette,
                        cx.listener(|this, _, window, _| {
                            this.trigger(TrayAction::OpenSettings, window);
                        }),
                    ))
                    // Subtle tonal breathing space instead of a 1px divider line
                    .child(div().h(px(4.0)).w_full())
                    .child(tray_menu_row(
                        TrayRowSpec {
                            id: "tray-item-quit",
                            icon: AppIcon::Close,
                            label: &t!("tray.quit"),
                            shortcut: None,
                            icon_color: palette.accent_red,
                            label_color: palette.accent_red,
                        },
                        &palette,
                        cx.listener(|this, _, window, _| {
                            this.trigger(TrayAction::Quit, window);
                        }),
                    )),
            )
    }
}

struct TrayRowSpec<'a> {
    id: &'static str,
    icon: AppIcon,
    label: &'a str,
    shortcut: Option<&'a str>,
    icon_color: gpui_kit::Hsla,
    label_color: gpui_kit::Hsla,
}

fn tray_menu_row(
    spec: TrayRowSpec<'_>,
    palette: &ThemePalette,
    on_click: impl Fn(&gpui_kit::MouseDownEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(spec.id)
        .cursor_pointer()
        .w_full()
        .h(px(30.0))
        .px(px(8.0))
        .rounded(px(8.0))
        .hover(|s| s.bg(palette.bg_hover))
        .on_mouse_down(MouseButton::Left, on_click)
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .min_w_0()
                .child(app_icon(spec.icon, 13.0, spec.icon_color))
                .child(
                    div()
                        .text_size(px(11.5))
                        .line_height(px(15.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(spec.label_color)
                        .truncate()
                        .child(spec.label.to_owned()),
                ),
        )
        .when_some(spec.shortcut, |el, sc| {
            el.child(
                div()
                    .flex_shrink_0()
                    .font_family(mono_font_family())
                    .text_size(px(9.5))
                    .line_height(px(12.0))
                    .text_color(palette.text_muted)
                    .child(sc.to_owned()),
            )
        })
        .into_any_element()
}
