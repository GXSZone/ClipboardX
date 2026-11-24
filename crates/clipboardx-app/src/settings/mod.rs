use std::rc::Rc;

use clipboardx_model::{
    AppConfig, Locale, PasteActionMode, RgbHex, SettingsTab, ThemeId, ThemeMode, UpdateChannel,
    VaultUnlockPolicy, WindowBlurBehavior,
};
use clipboardx_ocr::InstalledOcrPlugin;
use clipboardx_platform::{
    DesktopSessionDiagnostics, DesktopSessionKind, list_running_desktop_apps, open_external_url,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, KeyDownEvent,
    PathPromptOptions, Render, SharedString, Subscription, Window, div, px,
};
use rust_i18n::t;

use crate::icons::{AppIcon, app_icon, brand_app_icon, input_shield, was_blind_drag_consumed};
use crate::state::{AppRuntime, OcrDownloadStatus, StorageBreakdown, UpdateStatus};
use crate::theme::{
    ThemePalette, hex_to_hsla, mono_font_family, resolve_theme_id, sync_component_theme,
    ui_font_family, ui_weight_bold,
};

const REPO_URL: &str = "https://github.com/GXSZone/ClipboardX";
const ISSUES_URL: &str = "https://github.com/GXSZone/ClipboardX/issues";
const NOTICES_URL: &str = "https://github.com/GXSZone/ClipboardX/blob/main/LICENSE-MIT";

const KEEP_DAYS_OPTIONS: &[u32] = &[7, 30, 90, 365];
const MAX_ITEMS_OPTIONS: &[u32] = &[500, 2000, 5000, 10000];
const MAX_MB_OPTIONS: &[u64] = &[10, 20, 50, 100];

type CloseCallback = Rc<dyn Fn(&mut Window, &mut App)>;
type SegmentClickHandler = Rc<dyn Fn(&gpui_kit::ClickEvent, &mut Window, &mut App)>;

/// Borderless 680x420 settings center (dark & light tonal hierarchy):
/// 152px calm left navigation rail + spacious `bg/surface` content pane with soft `bg/window` groups.
pub struct SettingsView {
    runtime: Rc<AppRuntime>,
    on_close: CloseCallback,
    focus_handle: FocusHandle,
    pub active_tab: SettingsTab,
    palette: ThemePalette,
    blacklist_input: Entity<InputState>,
    blacklist_adding: bool,
    running_apps: Vec<String>,
    cbx_pass_input: Entity<InputState>,
    cbx_pass_expanded: bool,
    storage_status: Option<String>,
    ocr_status: Option<String>,
    cached_total_count: u64,
    cached_storage_usage: StorageBreakdown,
    cached_ocr_plugins: Vec<InstalledOcrPlugin>,
    cached_session_diag: DesktopSessionDiagnostics,
    cached_writable_dir: bool,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SettingsView {
    pub fn new(
        runtime: Rc<AppRuntime>,
        on_close: CloseCallback,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let cfg = runtime.snapshot_config();
        let theme_id = resolve_theme_id(&cfg, window.appearance());
        let palette = ThemePalette::from_theme_id(theme_id);
        sync_component_theme(&palette, Some(window), cx);

        let focus_handle = cx.focus_handle();
        let blacklist_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("settings.vault_privacy.blacklist_placeholder").to_string())
        });
        let cbx_pass_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(t!("settings.storage.cbx_pass_placeholder").to_string())
        });

        let sub = cx.subscribe_in(&blacklist_input, window, |this, state, ev, win, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                let raw = state.read(cx).value().to_string();
                let trimmed = raw.trim().to_owned();
                if !trimmed.is_empty() {
                    this.runtime.update_config(|cfg| {
                        if !cfg
                            .app_blacklist
                            .iter()
                            .any(|x| x.eq_ignore_ascii_case(&trimmed))
                        {
                            cfg.app_blacklist.push(trimmed);
                        }
                    });
                    state.update(cx, |s, cx| s.set_value("", win, cx));
                    this.blacklist_adding = false;
                    cx.notify();
                }
            }
        });

        let cached_session_diag = runtime.session_diagnostics();
        let cached_writable_dir = runtime.can_self_update();

        Self {
            runtime,
            on_close,
            focus_handle,
            active_tab: SettingsTab::General,
            palette,
            blacklist_input,
            blacklist_adding: false,
            running_apps: Vec::new(),
            cbx_pass_input,
            cbx_pass_expanded: false,
            storage_status: None,
            ocr_status: None,
            cached_total_count: 0,
            cached_storage_usage: StorageBreakdown::default(),
            cached_ocr_plugins: Vec::new(),
            cached_session_diag,
            cached_writable_dir,
            _subscriptions: vec![sub],
        }
    }

    pub fn focus_self(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    /// Refreshes disk/DB metrics for the currently active tab outside VSync animation frames.
    pub fn refresh_active_tab_metrics(&mut self) {
        self.refresh_tab_metrics(self.active_tab);
    }

    fn refresh_tab_metrics(&mut self, tab: SettingsTab) {
        match tab {
            SettingsTab::StorageBackup => {
                self.cached_total_count = self.runtime.service.total_count().unwrap_or(0);
                self.cached_storage_usage = self.runtime.storage_breakdown();
            }
            SettingsTab::OcrPlugins => {
                self.cached_ocr_plugins = self.runtime.list_ocr_plugins();
            }
            SettingsTab::AboutUpdates => {
                self.cached_session_diag = self.runtime.session_diagnostics();
            }
            _ => {}
        }
    }

    /// Updates the rendered theme palette smoothly during the 280ms theme cross-fade.
    pub fn set_animated_palette(&mut self, palette: ThemePalette, cx: &mut Context<'_, Self>) {
        self.palette = palette;
        cx.notify();
    }

    pub fn refresh_locale_placeholders(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.blacklist_input.update(cx, |state, cx| {
            state.set_placeholder(
                t!("settings.vault_privacy.blacklist_placeholder").to_string(),
                window,
                cx,
            );
        });
        self.cbx_pass_input.update(cx, |state, cx| {
            state.set_placeholder(
                t!("settings.storage.cbx_pass_placeholder").to_string(),
                window,
                cx,
            );
        });
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if ev.keystroke.key.as_str() == "escape" {
            if self.blacklist_adding {
                self.blacklist_adding = false;
                cx.notify();
                cx.stop_propagation();
                return;
            }
            (self.on_close)(window, cx);
            cx.stop_propagation();
        }
    }

    fn open_blacklist_app_picker(&mut self, cx: &mut Context<'_, Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SharedString::from(
                t!("settings.vault_privacy.blacklist_browse_prompt").to_string(),
            )),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next()
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                let name = stem.trim().to_owned();
                if !name.is_empty() {
                    let _ = this.update(cx, |view, cx| {
                        view.runtime.update_config(|cfg| {
                            if !cfg
                                .app_blacklist
                                .iter()
                                .any(|x| x.eq_ignore_ascii_case(&name))
                            {
                                cfg.app_blacklist.push(name.clone());
                            }
                        });
                        view.blacklist_adding = false;
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn open_ocr_zip_picker(&mut self, cx: &mut Context<'_, Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SharedString::from(
                t!("settings.ocr.choose_zip_prompt").to_string(),
            )),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |view, cx| {
                    let path_str = path.to_string_lossy();
                    match view.runtime.install_ocr_plugin(&path_str, None) {
                        Ok(p) => {
                            view.ocr_status = Some(
                                t!(
                                    "settings.ocr.installed_ok",
                                    name = p.manifest.name,
                                    version = p.manifest.version
                                )
                                .to_string(),
                            );
                            view.cached_ocr_plugins = view.runtime.list_ocr_plugins();
                        }
                        Err(err) => {
                            view.ocr_status = Some(err.to_string());
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn open_cbx_export_picker(&mut self, cx: &mut Context<'_, Self>) {
        let backup_dir = self.runtime.backup_dir();
        let rx = cx.prompt_for_new_path(&backup_dir, Some("clipboardx-backup.cbx"));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                let _ = this.update(cx, |view, cx| {
                    let pass_str = view.cbx_pass_input.read(cx).value().to_string();
                    let pass_opt = (!pass_str.is_empty()).then_some(pass_str.as_bytes());
                    match view.runtime.export_cbx(&path, pass_opt) {
                        Ok(stats) => {
                            view.storage_status = Some(
                                t!(
                                    "settings.storage.cbx_export_ok",
                                    clips = stats.history_exported,
                                    groups = stats.groups_exported,
                                    vault = stats.vault_exported
                                )
                                .to_string(),
                            );
                        }
                        Err(err) => {
                            view.storage_status = Some(err.to_string());
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn open_cbx_import_picker(&mut self, cx: &mut Context<'_, Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SharedString::from(
                t!("settings.storage.cbx_choose_import").to_string(),
            )),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |view, cx| {
                    let pass_str = view.cbx_pass_input.read(cx).value().to_string();
                    let pass_opt = (!pass_str.is_empty()).then_some(pass_str.as_bytes());
                    match view.runtime.import_cbx(&path, pass_opt) {
                        Ok(stats) => {
                            view.storage_status = Some(
                                t!(
                                    "settings.storage.cbx_import_ok",
                                    imported = stats.history_imported,
                                    skipped = stats.history_skipped_duplicates,
                                    vault = stats.vault_imported
                                )
                                .to_string(),
                            );
                            view.refresh_tab_metrics(SettingsTab::StorageBackup);
                        }
                        Err(err) => {
                            view.storage_status = Some(err.to_string());
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let palette = self.palette.clone();
        let cfg = self.runtime.snapshot_config();

        let (title, content) = match self.active_tab {
            SettingsTab::General => (
                t!("settings.general.title").to_string(),
                self.render_general(&cfg, &palette, cx),
            ),
            SettingsTab::Appearance => (
                t!("settings.appearance.title").to_string(),
                self.render_appearance(&cfg, &palette, cx),
            ),
            SettingsTab::VaultPrivacy => (
                t!("settings.vault_privacy.title").to_string(),
                self.render_vault_privacy(&cfg, &palette, cx),
            ),
            SettingsTab::StorageBackup => (
                t!("settings.storage.title").to_string(),
                self.render_storage_backup(&cfg, &palette, cx),
            ),
            SettingsTab::OcrPlugins => (
                t!("settings.ocr.title").to_string(),
                self.render_ocr_plugins(&cfg, &palette, cx),
            ),
            SettingsTab::AboutUpdates => (
                t!("settings.about.title").to_string(),
                self.render_about_updates(&cfg, &palette, cx),
            ),
        };

        let rt_zh = Rc::clone(&self.runtime);
        let rt_en = Rc::clone(&self.runtime);

        div()
            .id("settings-root")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .size_full()
            .font_family(ui_font_family())
            .text_color(palette.text_primary)
            .p(px(8.0))
            .flex()
            .gap(px(8.0))
            .overflow_hidden()
            // Left 152px navigation rail
            .child(
                div()
                    .w(px(152.0))
                    .h_full()
                    .flex_shrink_0()
                    .pt(px(10.0))
                    .px(px(6.0))
                    .pb(px(6.0))
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        div()
                            .id("settings-nav-brand-drag")
                            .pl(px(8.0))
                            .pb(px(12.0))
                            .flex()
                            .items_center()
                            .gap(px(7.0))
                            .child(brand_app_icon(22.0))
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .line_height(px(16.0))
                                    .font_weight(ui_weight_bold())
                                    .text_color(palette.text_primary)
                                    .child("ClipboardX"),
                            ),
                    )
                    .children(SettingsTab::ALL.iter().copied().map(|tab| {
                        let active = self.active_tab == tab;
                        div()
                            .id(SharedString::from(format!(
                                "settings-nav-{}",
                                tab.i18n_key()
                            )))
                            .cursor_pointer()
                            .w_full()
                            .h(px(30.0))
                            .rounded(px(8.0))
                            .bg(if active {
                                palette.bg_selected
                            } else {
                                gpui_kit::transparent_black()
                            })
                            .hover(|s| if active { s } else { s.bg(palette.bg_hover) })
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .text_size(px(12.0))
                            .line_height(px(16.0))
                            .font_weight(if active {
                                FontWeight::MEDIUM
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if active {
                                palette.accent_blue
                            } else {
                                palette.text_secondary
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !was_blind_drag_consumed() && this.active_tab != tab {
                                    this.active_tab = tab;
                                    this.storage_status = None;
                                    this.ocr_status = None;
                                    this.blacklist_adding = false;
                                    this.refresh_tab_metrics(tab);
                                    cx.notify();
                                }
                            }))
                            .child(t!(tab.i18n_key()).to_string())
                    }))
                    .child(div().id("settings-nav-drag-spacer").flex_1().w_full())
                    .child(
                        div()
                            .w_full()
                            .h(px(28.0))
                            .rounded(px(8.0))
                            .bg(palette.bg_surface)
                            .p(px(3.0))
                            .flex()
                            .items_center()
                            .gap(px(3.0))
                            .child(lang_pill(
                                "lang-zh",
                                "中文",
                                cfg.locale == Locale::ZhCn,
                                &palette,
                                cx.listener(move |this, _, win, cx| {
                                    rt_zh.update_config(|c| c.locale = Locale::ZhCn);
                                    this.refresh_locale_placeholders(win, cx);
                                }),
                            ))
                            .child(lang_pill(
                                "lang-en",
                                "EN",
                                cfg.locale == Locale::EnUs,
                                &palette,
                                cx.listener(move |this, _, win, cx| {
                                    rt_en.update_config(|c| c.locale = Locale::EnUs);
                                    this.refresh_locale_placeholders(win, cx);
                                }),
                            )),
                    ),
            )
            // Right Clean Content Surface (bg/surface, r=12, pad=[18,22,16,22])
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .rounded(px(12.0))
                    .bg(palette.bg_surface)
                    .pt(px(18.0))
                    .px(px(22.0))
                    .pb(px(16.0))
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .overflow_hidden()
                    // Single-line minimal header
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .id("settings-header-drag-title")
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(15.0))
                                    .line_height(px(20.0))
                                    .font_weight(ui_weight_bold())
                                    .text_color(palette.text_primary)
                                    .truncate()
                                    .child(title),
                            )
                            .child(
                                div()
                                    .id("settings-back-btn")
                                    .cursor_pointer()
                                    .flex_shrink_0()
                                    .size(px(26.0))
                                    .rounded(px(7.0))
                                    .bg(gpui_kit::transparent_black())
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, _| {
                                        crate::icons::reset_blind_drag_click_guard();
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if !was_blind_drag_consumed() {
                                            (this.on_close)(window, cx);
                                        }
                                    }))
                                    .child(app_icon(AppIcon::Close, 13.0, palette.text_secondary)),
                            ),
                    )
                    .child(
                        div()
                            .id("settings-content-scroll")
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .overflow_y_scroll()
                            .child(div().w_full().child(content)),
                    ),
            )
    }
}

impl SettingsView {
    /// Renders General, Hotkeys, and Floating Capsule settings.
    fn render_general(
        &self,
        cfg: &AppConfig,
        palette: &ThemePalette,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let rt_blur = Rc::clone(&self.runtime);
        let rt_paste = Rc::clone(&self.runtime);
        let rt_startup = Rc::clone(&self.runtime);
        let rt_remember = Rc::clone(&self.runtime);

        div()
            .flex()
            .flex_col()
            .gap(px(11.0))
            .child(section_header("全局快捷键", None, palette))
            .child(
                soft_group(10.0, 14.0, 8.0, palette)
                    .child(hotkey_row(
                        &t!("settings.general.hotkeys_toggle"),
                        &cfg.toggle_hotkey,
                        palette,
                    ))
                    .child(hotkey_row(
                        &t!("settings.general.hotkeys_compact"),
                        &cfg.compact_hotkey,
                        palette,
                    ))
                    .child(hotkey_row(
                        &t!("settings.general.hotkeys_pause"),
                        &cfg.pause_hotkey,
                        palette,
                    )),
            )
            .child(section_header(
                &t!("settings.general.blur_behavior"),
                None,
                palette,
            ))
            .child(clean_segment_bar(
                palette.bg_window,
                vec![
                    (
                        "贴右迷你胶囊".to_owned(),
                        cfg.blur_behavior == WindowBlurBehavior::DockRightCapsule,
                        Rc::new(cx.listener({
                            let rt = Rc::clone(&rt_blur);
                            move |_, _, _, cx| {
                                rt.update_config(|c| {
                                    c.blur_behavior = WindowBlurBehavior::DockRightCapsule
                                });
                                cx.notify();
                            }
                        })),
                    ),
                    (
                        "自动隐藏".to_owned(),
                        cfg.blur_behavior == WindowBlurBehavior::AutoHide,
                        Rc::new(cx.listener(move |_, _, _, cx| {
                            rt_blur
                                .update_config(|c| c.blur_behavior = WindowBlurBehavior::AutoHide);
                            cx.notify();
                        })),
                    ),
                ],
                palette,
            ))
            .child(
                soft_group(10.0, 14.0, 10.0, palette)
                    .child(clean_toggle_row(
                        "toggle-paste-auto",
                        "单击条目直接粘贴",
                        "左键单击立即粘贴至原焦点窗口，关闭后仅写入剪贴板",
                        cfg.paste_mode == PasteActionMode::AutoPaste,
                        palette,
                        cx.listener(move |_, _, _, cx| {
                            rt_paste.update_config(|c| {
                                c.paste_mode = if c.paste_mode == PasteActionMode::AutoPaste {
                                    PasteActionMode::CopyOnly
                                } else {
                                    PasteActionMode::AutoPaste
                                };
                            });
                            cx.notify();
                        }),
                    ))
                    .child(clean_toggle_row(
                        "toggle-launch-startup",
                        &t!("settings.general.launch_startup"),
                        &t!("settings.general.launch_startup_sub"),
                        cfg.launch_at_startup,
                        palette,
                        cx.listener(move |_, _, _, cx| {
                            rt_startup
                                .update_config(|c| c.launch_at_startup = !c.launch_at_startup);
                            cx.notify();
                        }),
                    ))
                    .child(clean_toggle_row(
                        "toggle-remember-compact",
                        &t!("settings.general.remember_compact"),
                        &t!("settings.general.remember_compact_sub"),
                        cfg.remember_compact_mode,
                        palette,
                        cx.listener(move |_, _, _, cx| {
                            rt_remember.update_config(|c| {
                                let next = !c.remember_compact_mode;
                                c.remember_compact_mode = next;
                                c.smooth_collapse_animation = next;
                            });
                            cx.notify();
                        }),
                    )),
            )
            .into_any_element()
    }

    /// Renders Appearance and dual-theme selection settings.
    fn render_appearance(
        &self,
        cfg: &AppConfig,
        palette: &ThemePalette,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let rt_mode = Rc::clone(&self.runtime);
        let rt_dark_1 = Rc::clone(&self.runtime);
        let rt_dark_2 = Rc::clone(&self.runtime);
        let rt_light_1 = Rc::clone(&self.runtime);
        let rt_light_2 = Rc::clone(&self.runtime);
        let rt_vib = Rc::clone(&self.runtime);

        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(clean_segment_bar(
                palette.bg_window,
                vec![
                    (
                        t!("settings.appearance.mode_system").to_string(),
                        cfg.theme_mode == ThemeMode::System,
                        Rc::new(cx.listener({
                            let rt = Rc::clone(&rt_mode);
                            move |_, _, _, cx| {
                                rt.update_config(|c| c.theme_mode = ThemeMode::System);
                                cx.notify();
                            }
                        })),
                    ),
                    (
                        t!("settings.appearance.mode_dark").to_string(),
                        cfg.theme_mode == ThemeMode::AlwaysDark,
                        Rc::new(cx.listener({
                            let rt = Rc::clone(&rt_mode);
                            move |_, _, _, cx| {
                                rt.update_config(|c| c.theme_mode = ThemeMode::AlwaysDark);
                                cx.notify();
                            }
                        })),
                    ),
                    (
                        t!("settings.appearance.mode_light").to_string(),
                        cfg.theme_mode == ThemeMode::AlwaysLight,
                        Rc::new(cx.listener(move |_, _, _, cx| {
                            rt_mode.update_config(|c| c.theme_mode = ThemeMode::AlwaysLight);
                            cx.notify();
                        })),
                    ),
                ],
                palette,
            ))
            .child(section_header("精选主题", None, palette))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .gap(px(10.0))
                            .child(theme_preview_card(
                                ThemeId::ZedOneDark,
                                "深色默认",
                                ThemeSwatch::new(0x14171D, 0x1B1F27, 0x3B82F6),
                                cfg.dark_theme == ThemeId::ZedOneDark,
                                palette,
                                cx.listener(move |_, _, _, cx| {
                                    rt_dark_1.update_config(|c| c.dark_theme = ThemeId::ZedOneDark);
                                    cx.notify();
                                }),
                            ))
                            .child(theme_preview_card(
                                ThemeId::CatppuccinMocha,
                                "深色柔和",
                                ThemeSwatch::new(0x1E1E2E, 0x313244, 0xCBA6F7),
                                cfg.dark_theme == ThemeId::CatppuccinMocha,
                                palette,
                                cx.listener(move |_, _, _, cx| {
                                    rt_dark_2
                                        .update_config(|c| c.dark_theme = ThemeId::CatppuccinMocha);
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .gap(px(10.0))
                            .child(theme_preview_card(
                                ThemeId::MinimalLight,
                                "浅色默认",
                                ThemeSwatch::new(0xEFF2F6, 0xFFFFFF, 0x2563EB),
                                cfg.light_theme == ThemeId::MinimalLight,
                                palette,
                                cx.listener(move |_, _, _, cx| {
                                    rt_light_1
                                        .update_config(|c| c.light_theme = ThemeId::MinimalLight);
                                    cx.notify();
                                }),
                            ))
                            .child(theme_preview_card(
                                ThemeId::CatppuccinLatte,
                                "浅色暖白",
                                ThemeSwatch::new(0xEFF1F5, 0xDCE0E8, 0x8839EF),
                                cfg.light_theme == ThemeId::CatppuccinLatte,
                                palette,
                                cx.listener(move |_, _, _, cx| {
                                    rt_light_2.update_config(|c| {
                                        c.light_theme = ThemeId::CatppuccinLatte
                                    });
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .child(soft_group(10.0, 14.0, 8.0, palette).child(clean_toggle_row(
                "toggle-vibrancy",
                "系统原生毛玻璃",
                "Mica / Vibrancy · 全站 0px 零描边色阶分层",
                cfg.window_vibrancy,
                palette,
                cx.listener(move |_, _, _, cx| {
                    rt_vib.update_config(|c| c.window_vibrancy = !c.window_vibrancy);
                    cx.notify();
                }),
            )))
            .into_any_element()
    }

    /// Renders Account Vault and Privacy Protection settings.
    fn render_vault_privacy(
        &self,
        cfg: &AppConfig,
        palette: &ThemePalette,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let rt_pol_1 = Rc::clone(&self.runtime);
        let rt_pol_2 = Rc::clone(&self.runtime);
        let rt_pol_3 = Rc::clone(&self.runtime);
        let rt_lock = Rc::clone(&self.runtime);
        let rt_conf = Rc::clone(&self.runtime);
        let rt_rest = Rc::clone(&self.runtime);

        div()
            .flex()
            .flex_col()
            .gap(px(11.0))
            .child(section_header(
                &t!("settings.vault_privacy.unlock_policy"),
                Some(
                    div()
                        .id("settings-vault-lock-now")
                        .cursor_pointer()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_secondary)
                        .hover(|s| s.text_color(palette.text_primary))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            if !was_blind_drag_consumed() {
                                rt_lock.vault.lock();
                                rt_lock.notify_ui(crate::state::UiEvent::HistoryUpdated);
                                cx.notify();
                            }
                        }))
                        .child(t!("settings.vault_privacy.lock_now").to_string())
                        .into_any_element(),
                ),
                palette,
            ))
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap(px(8.0))
                    .child(policy_option_card(
                        "pol-trusted",
                        "信任本机",
                        "免密 · 托管密钥环",
                        cfg.vault_unlock_policy == VaultUnlockPolicy::TrustedDeviceNoPrompt,
                        palette,
                        cx.listener(move |_, _, _, cx| {
                            rt_pol_1.update_config(|c| {
                                c.vault_unlock_policy = VaultUnlockPolicy::TrustedDeviceNoPrompt
                            });
                            cx.notify();
                        }),
                    ))
                    .child(policy_option_card(
                        "pol-timeout",
                        &format!("{} 分钟锁定", cfg.vault_timeout_minutes),
                        "按需主密码解锁",
                        cfg.vault_unlock_policy == VaultUnlockPolicy::PromptWithTimeout,
                        palette,
                        cx.listener(move |_, _, _, cx| {
                            rt_pol_2.update_config(|c| {
                                c.vault_unlock_policy = VaultUnlockPolicy::PromptWithTimeout
                            });
                            cx.notify();
                        }),
                    ))
                    .child(policy_option_card(
                        "pol-session",
                        "单次会话",
                        "每次开机仅输一次",
                        cfg.vault_unlock_policy == VaultUnlockPolicy::OncePerSession,
                        palette,
                        cx.listener(move |_, _, _, cx| {
                            rt_pol_3.update_config(|c| {
                                c.vault_unlock_policy = VaultUnlockPolicy::OncePerSession
                            });
                            cx.notify();
                        }),
                    )),
            )
            .child(
                soft_group(10.0, 14.0, 10.0, palette)
                    .child(clean_toggle_row(
                        "toggle-ignore-confidential",
                        &t!("settings.vault_privacy.ignore_confidential"),
                        &t!("settings.vault_privacy.ignore_confidential_sub"),
                        cfg.ignore_confidential_clipboard,
                        palette,
                        cx.listener(move |_, _, _, cx| {
                            rt_conf.update_config(|c| {
                                c.ignore_confidential_clipboard = !c.ignore_confidential_clipboard
                            });
                            cx.notify();
                        }),
                    ))
                    .child(clean_toggle_row(
                        "toggle-restore-after-fill",
                        &t!("settings.vault_privacy.restore_clipboard"),
                        &t!("settings.vault_privacy.restore_clipboard_sub"),
                        cfg.restore_clipboard_after_autofill,
                        palette,
                        cx.listener(move |_, _, _, cx| {
                            rt_rest.update_config(|c| {
                                c.restore_clipboard_after_autofill =
                                    !c.restore_clipboard_after_autofill
                            });
                            cx.notify();
                        }),
                    )),
            )
            .child(section_header(
                "忽略应用黑名单",
                Some(
                    div()
                        .id("bl-toggle-add-btn")
                        .cursor_pointer()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.accent_blue)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if !was_blind_drag_consumed() {
                                this.blacklist_adding = !this.blacklist_adding;
                                if this.blacklist_adding {
                                    this.running_apps = list_running_desktop_apps();
                                    let fh = this.blacklist_input.read(cx).focus_handle(cx);
                                    window.focus(&fh, cx);
                                }
                                cx.notify();
                            }
                        }))
                        .child("+ 选择应用")
                        .into_any_element(),
                ),
                palette,
            ))
            .child(
                soft_group(10.0, 12.0, 8.0, palette)
                    .when(self.blacklist_adding, |el| {
                        let available_apps: Vec<String> = self
                            .running_apps
                            .iter()
                            .filter(|app| {
                                !cfg.app_blacklist
                                    .iter()
                                    .any(|b| b.eq_ignore_ascii_case(app))
                            })
                            .cloned()
                            .collect();
                        el.child(
                            div()
                                .w_full()
                                .rounded(px(8.0))
                                .bg(palette.bg_surface)
                                .p(px(8.0))
                                .flex()
                                .flex_col()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .w_full()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .h(px(24.0))
                                                .rounded(px(6.0))
                                                .bg(palette.bg_window)
                                                .px(px(8.0))
                                                .flex()
                                                .items_center()
                                                .child(input_shield(
                                                    "bl-input-shield",
                                                    Input::new(&self.blacklist_input)
                                                        .appearance(false)
                                                        .bordered(false)
                                                        .focus_bordered(false)
                                                        .text_size(px(10.0))
                                                        .into_any_element(),
                                                )),
                                        )
                                        .child(
                                            div()
                                                .id("bl-browse-app-btn")
                                                .cursor_pointer()
                                                .flex_shrink_0()
                                                .h(px(24.0))
                                                .px(px(8.0))
                                                .rounded(px(6.0))
                                                .bg(palette.bg_window)
                                                .hover(|s| s.bg(palette.bg_hover))
                                                .flex()
                                                .items_center()
                                                .text_size(px(10.0))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(palette.accent_blue)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    if !was_blind_drag_consumed() {
                                                        this.open_blacklist_app_picker(cx);
                                                    }
                                                }))
                                                .child(
                                                    t!("settings.vault_privacy.blacklist_browse_app")
                                                        .to_string(),
                                                ),
                                        ),
                                )
                                .when(!available_apps.is_empty(), |el| {
                                    el.child(
                                        div().w_full().flex().flex_wrap().gap(px(5.0)).children(
                                            available_apps.into_iter().take(12).enumerate().map(
                                                |(idx, app)| {
                                                    let rt_add = Rc::clone(&self.runtime);
                                                    let app_add = app.clone();
                                                    div()
                                                        .id(SharedString::from(format!(
                                                            "bl-pick-{idx}"
                                                        )))
                                                        .cursor_pointer()
                                                        .px(px(7.0))
                                                        .py(px(3.0))
                                                        .rounded(px(6.0))
                                                        .bg(palette.bg_window)
                                                        .hover(|s| s.bg(palette.bg_selected))
                                                        .text_size(px(10.0))
                                                        .text_color(palette.text_primary)
                                                        .on_click(cx.listener(
                                                            move |this, _, _, cx| {
                                                                if was_blind_drag_consumed() {
                                                                    return;
                                                                }
                                                                let name = app_add.clone();
                                                                rt_add.update_config(|c| {
                                                                    if !c
                                                                        .app_blacklist
                                                                        .iter()
                                                                        .any(|x| {
                                                                            x.eq_ignore_ascii_case(
                                                                                &name,
                                                                            )
                                                                        })
                                                                    {
                                                                        c.app_blacklist.push(name);
                                                                    }
                                                                });
                                                                this.blacklist_adding = false;
                                                                cx.notify();
                                                            },
                                                        ))
                                                        .child(format!("+ {app}"))
                                                },
                                            ),
                                        ),
                                    )
                                }),
                        )
                    })
                    .child(
                        div().w_full().flex().flex_wrap().gap(px(6.0)).children(
                            cfg.app_blacklist.iter().enumerate().map(|(idx, app)| {
                                let app_cmp = app.clone();
                                let rt_rm = Rc::clone(&self.runtime);
                                div()
                                    .id(SharedString::from(format!("bl-chip-{idx}")))
                                    .cursor_pointer()
                                    .px(px(9.0))
                                    .py(px(4.0))
                                    .rounded(px(6.0))
                                    .bg(palette.bg_surface)
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .text_size(px(10.5))
                                    .line_height(px(13.0))
                                    .text_color(palette.text_primary)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        if !was_blind_drag_consumed() {
                                            rt_rm.update_config(|c| {
                                                c.app_blacklist.retain(|item| item != &app_cmp);
                                            });
                                            cx.notify();
                                        }
                                    }))
                                    .child(format!("{app} ×"))
                            }),
                        ),
                    ),
            )
            .into_any_element()
    }

    /// Renders Storage Breakdown and encrypted `.cbx` archive backup settings.
    fn render_storage_backup(
        &self,
        cfg: &AppConfig,
        palette: &ThemePalette,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let count = self.cached_total_count;
        let usage = self.cached_storage_usage;
        let total = usage.total_bytes().max(1) as f32;
        let db_ratio = (usage.db_bytes as f32 / total).clamp(0.08, 0.84);
        let blob_ratio = (usage.blobs_bytes as f32 / total).clamp(0.04, 0.84);
        let thumb_ratio = (usage.thumbs_bytes as f32 / total).clamp(0.04, 0.84);
        let sum_ratio = db_ratio + blob_ratio + thumb_ratio;
        let (db_pct, blob_pct, thumb_pct) = (
            db_ratio / sum_ratio,
            blob_ratio / sum_ratio,
            thumb_ratio / sum_ratio,
        );

        let rt_prune = Rc::clone(&self.runtime);
        let rt_days = Rc::clone(&self.runtime);
        let rt_items = Rc::clone(&self.runtime);
        let rt_mb = Rc::clone(&self.runtime);
        let retention = cfg.retention.clone();
        let current_mb = (cfg.retention.max_item_bytes / (1024 * 1024)).max(1);

        div()
            .flex()
            .flex_col()
            .gap(px(11.0))
            .child(section_header(
                &t!("settings.storage.total_usage"),
                Some(
                    div()
                        .text_size(px(10.5))
                        .line_height(px(14.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.accent_blue)
                        .child(format!(
                            "{} · {}",
                            format_bytes(usage.total_bytes()),
                            t!("search.count_items", count = count)
                        ))
                        .into_any_element(),
                ),
                palette,
            ))
            .child(
                soft_group(12.0, 14.0, 9.0, palette)
                    .child({
                        let segments: Vec<(f32, gpui_kit::Hsla)> = [
                            (
                                usage.db_bytes > 0 || usage.total_bytes() == 0,
                                db_pct,
                                palette.accent_blue,
                            ),
                            (usage.blobs_bytes > 0, blob_pct, palette.accent_purple),
                            (usage.thumbs_bytes > 0, thumb_pct, palette.accent_green),
                        ]
                        .into_iter()
                        .filter(|(show, _, _)| *show)
                        .map(|(_, pct, color)| (pct, color))
                        .collect();
                        let seg_count = segments.len();
                        let mut bar = div()
                            .w_full()
                            .h(px(7.0))
                            .rounded(px(999.0))
                            .bg(palette.bg_surface)
                            .overflow_hidden()
                            .flex()
                            .gap(px(2.0));
                        for (idx, (pct, color)) in segments.into_iter().enumerate() {
                            let is_first = idx == 0;
                            let is_last = idx + 1 == seg_count;
                            let mut seg = div()
                                .w(gpui_kit::relative(pct))
                                .min_w(px(8.0))
                                .h_full()
                                .bg(color)
                                .rounded(px(2.0));
                            if is_first && is_last {
                                seg = seg.rounded(px(999.0));
                            } else if is_first {
                                seg = seg.rounded_l(px(999.0));
                            } else if is_last {
                                seg = seg.rounded_r(px(999.0));
                            }
                            bar = bar.child(seg);
                        }
                        bar
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(14.0))
                            .child(storage_legend_dot(
                                palette.accent_blue,
                                &t!(
                                    "settings.storage.legend_db",
                                    size = format_bytes(usage.db_bytes)
                                ),
                                palette,
                            ))
                            .child(storage_legend_dot(
                                palette.accent_purple,
                                &t!(
                                    "settings.storage.legend_blobs",
                                    size = format_bytes(usage.blobs_bytes)
                                ),
                                palette,
                            ))
                            .child(storage_legend_dot(
                                palette.accent_green,
                                &t!(
                                    "settings.storage.legend_thumbs",
                                    size = format_bytes(usage.thumbs_bytes)
                                ),
                                palette,
                            )),
                    ),
            )
            .child(section_header(
                "自动淘汰策略（置顶与保险箱豁免）",
                Some(
                    div()
                        .id("settings-prune-now")
                        .cursor_pointer()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_secondary)
                        .hover(|s| s.text_color(palette.text_primary))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !was_blind_drag_consumed() {
                                let removed =
                                    rt_prune.service.prune_retention(&retention).unwrap_or(0);
                                this.storage_status = Some(
                                    t!("settings.storage.pruned_msg", count = removed).to_string(),
                                );
                                this.refresh_tab_metrics(SettingsTab::StorageBackup);
                                rt_prune.notify_ui(crate::state::UiEvent::HistoryUpdated);
                                cx.notify();
                            }
                        }))
                        .child(t!("settings.storage.prune_now").to_string())
                        .into_any_element(),
                ),
                palette,
            ))
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap(px(8.0))
                    .child(retention_selector_cell(
                        "ret-days",
                        &t!("settings.storage.keep_days_label"),
                        &t!(
                            "settings.storage.keep_days_val",
                            days = cfg.retention.max_days
                        ),
                        palette,
                        cx.listener(move |_, _, _, cx| {
                            rt_days.update_config(|c| {
                                c.retention.max_days =
                                    next_cycle(KEEP_DAYS_OPTIONS, c.retention.max_days);
                            });
                            cx.notify();
                        }),
                    ))
                    .child(retention_selector_cell(
                        "ret-items",
                        &t!("settings.storage.max_items_label"),
                        &t!(
                            "settings.storage.max_items_val",
                            items = format_thousands(cfg.retention.max_items)
                        ),
                        palette,
                        cx.listener(move |_, _, _, cx| {
                            rt_items.update_config(|c| {
                                c.retention.max_items =
                                    next_cycle(MAX_ITEMS_OPTIONS, c.retention.max_items);
                            });
                            cx.notify();
                        }),
                    ))
                    .child(retention_selector_cell(
                        "ret-mb",
                        &t!("settings.storage.max_size_label"),
                        &t!("settings.storage.max_size_val", mb = current_mb),
                        palette,
                        cx.listener(move |_, _, _, cx| {
                            rt_mb.update_config(|c| {
                                let mb = next_cycle(
                                    MAX_MB_OPTIONS,
                                    (c.retention.max_item_bytes / (1024 * 1024)).max(1),
                                );
                                c.retention.max_item_bytes = mb * 1024 * 1024;
                            });
                            cx.notify();
                        }),
                    )),
            )
            .child(section_header(
                ".cbx 离线加密归档",
                Some(
                    div()
                        .id("cbx-pass-toggle")
                        .cursor_pointer()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if self.cbx_pass_expanded {
                            palette.accent_blue
                        } else {
                            palette.text_secondary
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            if !was_blind_drag_consumed() {
                                this.cbx_pass_expanded = !this.cbx_pass_expanded;
                                if this.cbx_pass_expanded {
                                    let fh = this.cbx_pass_input.read(cx).focus_handle(cx);
                                    window.focus(&fh, cx);
                                }
                                cx.notify();
                            }
                        }))
                        .child(t!("settings.storage.cbx_pass_btn").to_string())
                        .into_any_element(),
                ),
                palette,
            ))
            .when(self.cbx_pass_expanded, |el| {
                el.child(
                    div()
                        .w_full()
                        .h(px(28.0))
                        .rounded(px(8.0))
                        .bg(palette.bg_window)
                        .px(px(10.0))
                        .flex()
                        .items_center()
                        .child(input_shield(
                            "cbx-pass-shield",
                            Input::new(&self.cbx_pass_input)
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false)
                                .text_size(px(10.5))
                                .into_any_element(),
                        )),
                )
            })
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("settings-cbx-export")
                            .cursor_pointer()
                            .flex_1()
                            .h(px(32.0))
                            .rounded(px(8.0))
                            .bg(palette.accent_blue)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(11.0))
                            .line_height(px(15.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(gpui_kit::rgb(0xFFFFFF))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !was_blind_drag_consumed() {
                                    this.open_cbx_export_picker(cx);
                                }
                            }))
                            .child(t!("settings.storage.cbx_export").to_string()),
                    )
                    .child(
                        div()
                            .id("settings-cbx-import")
                            .cursor_pointer()
                            .flex_1()
                            .h(px(32.0))
                            .rounded(px(8.0))
                            .bg(palette.bg_window)
                            .hover(|s| s.bg(palette.bg_hover))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(11.0))
                            .line_height(px(15.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.text_primary)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !was_blind_drag_consumed() {
                                    this.open_cbx_import_picker(cx);
                                }
                            }))
                            .child(t!("settings.storage.cbx_import").to_string()),
                    ),
            )
            .when_some(self.storage_status.clone(), |el, msg| {
                el.child(
                    div()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .text_color(palette.accent_green)
                        .truncate()
                        .child(msg),
                )
            })
            .into_any_element()
    }

    /// Renders OCR Engine and offline plugin management settings.
    fn render_ocr_plugins(
        &self,
        cfg: &AppConfig,
        palette: &ThemePalette,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let rt_ocr = Rc::clone(&self.runtime);
        let rt_native = Rc::clone(&self.runtime);
        let rt_dl = Rc::clone(&self.runtime);
        let plugins = self.cached_ocr_plugins.clone();
        let dl_status = self.runtime.ocr_download_status();
        let native_badge_name = self.runtime.native_ocr_short_name();
        let using_native = cfg.active_ocr_plugin.is_none();

        let mut engine_group = soft_group(12.0, 14.0, 12.0, palette).child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .line_height(px(16.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.text_primary)
                                .child(t!("settings.ocr.auto_native").to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .line_height(px(14.0))
                                .text_color(palette.text_muted)
                                .truncate()
                                .child(t!("settings.ocr.auto_native_sub").to_string()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .flex_shrink_0()
                        .child(
                            div()
                                .text_size(px(10.0))
                                .line_height(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.accent_green)
                                .child(format!("{native_badge_name} · 就绪")),
                        )
                        .when(!using_native, |el| {
                            el.child(
                                div()
                                    .id("ocr-use-native-btn")
                                    .cursor_pointer()
                                    .text_size(px(10.0))
                                    .text_color(palette.accent_blue)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        if !was_blind_drag_consumed() {
                                            rt_native.update_config(|c| {
                                                c.active_ocr_plugin = None;
                                            });
                                            cx.notify();
                                        }
                                    }))
                                    .child(t!("settings.ocr.use_native").to_string()),
                            )
                        })
                        .child(toggle_switch(
                            "toggle-auto-ocr",
                            cfg.auto_ocr_native,
                            palette,
                            cx.listener(move |_, _, _, cx| {
                                rt_ocr.update_config(|c| c.auto_ocr_native = !c.auto_ocr_native);
                                cx.notify();
                            }),
                        )),
                ),
        );

        if plugins.is_empty() {
            let (btn_bg, btn_fg, btn_label, is_busy) = match &dl_status {
                OcrDownloadStatus::Idle => (
                    palette.accent_blue,
                    gpui_kit::rgb(0xFFFFFF).into(),
                    t!("settings.ocr.download_load_btn").to_string(),
                    false,
                ),
                OcrDownloadStatus::Downloading {
                    stage,
                    downloaded_bytes,
                    total_bytes,
                } => {
                    let label = if stage == "connecting" {
                        t!("settings.ocr.connecting_btn").to_string()
                    } else if stage == "verifying" || stage == "extracting" {
                        t!("settings.ocr.verifying_btn").to_string()
                    } else if let Some(total) = total_bytes.filter(|&v| v > 0) {
                        let pct = ((*downloaded_bytes as f64 / total as f64) * 100.0)
                            .clamp(0.0, 100.0) as u32;
                        t!("settings.ocr.downloading_btn", pct = pct).to_string()
                    } else {
                        t!("settings.ocr.connecting_btn").to_string()
                    };
                    (palette.bg_selected, palette.accent_blue, label, true)
                }
                OcrDownloadStatus::Error(_) => (
                    palette.accent_blue,
                    gpui_kit::rgb(0xFFFFFF).into(),
                    t!("settings.ocr.retry_download_btn").to_string(),
                    false,
                ),
            };

            engine_group = engine_group.child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .line_height(px(16.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.text_primary)
                                    .child(t!("settings.ocr.rapid_title").to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .line_height(px(14.0))
                                    .text_color(palette.text_muted)
                                    .truncate()
                                    .child(format!(
                                        "{} · {}",
                                        t!("settings.ocr.rapid_meta"),
                                        t!("settings.ocr.rapid_sub")
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .id("ocr-download-primary-btn")
                            .when(!is_busy, |el| el.cursor_pointer())
                            .flex_shrink_0()
                            .px(px(12.0))
                            .py(px(5.0))
                            .rounded(px(7.0))
                            .bg(btn_bg)
                            .text_size(px(11.0))
                            .line_height(px(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(btn_fg)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !is_busy && !was_blind_drag_consumed() {
                                    this.ocr_status = None;
                                    rt_dl.download_rapidocr_async();
                                    cx.notify();
                                }
                            }))
                            .child(btn_label),
                    ),
            );

            if let OcrDownloadStatus::Downloading {
                downloaded_bytes,
                total_bytes,
                ..
            } = &dl_status
            {
                let ratio = total_bytes
                    .filter(|&v| v > 0)
                    .map(|total| (*downloaded_bytes as f32 / total as f32).clamp(0.06, 1.0))
                    .unwrap_or(0.18);
                engine_group = engine_group.child(
                    div()
                        .w_full()
                        .h(px(5.0))
                        .rounded(px(999.0))
                        .bg(palette.bg_surface)
                        .overflow_hidden()
                        .child(
                            div()
                                .w(gpui_kit::relative(ratio))
                                .min_w(px(6.0))
                                .h_full()
                                .rounded(px(999.0))
                                .bg(palette.accent_blue),
                        ),
                );
            } else if let OcrDownloadStatus::Error(err_msg) = &dl_status {
                engine_group = engine_group.child(
                    div()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .text_color(palette.accent_red)
                        .truncate()
                        .child(err_msg.clone()),
                );
            } else if let Some(msg) = &self.ocr_status {
                engine_group = engine_group.child(
                    div()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .text_color(palette.text_muted)
                        .truncate()
                        .child(msg.clone()),
                );
            }
        } else {
            for (idx, p) in plugins.into_iter().enumerate() {
                let id_activate = p.manifest.id.clone();
                let id_remove = p.manifest.id.clone();
                let active = cfg.active_ocr_plugin.as_deref() == Some(p.manifest.id.as_str());
                let rt_act = Rc::clone(&self.runtime);
                let rt_rem = Rc::clone(&self.runtime);
                engine_group = engine_group.child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .line_height(px(16.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(palette.text_primary)
                                        .truncate()
                                        .child(format!(
                                            "{} v{}",
                                            p.manifest.name, p.manifest.version
                                        )),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .line_height(px(13.0))
                                        .text_color(palette.text_muted)
                                        .truncate()
                                        .child(if p.manifest.description.is_empty() {
                                            t!("settings.ocr.rapid_sub").to_string()
                                        } else {
                                            p.manifest.description.clone()
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .flex_shrink_0()
                                .child(
                                    div()
                                        .id(SharedString::from(format!("ocr-use-{idx}")))
                                        .cursor_pointer()
                                        .px(px(9.0))
                                        .py(px(4.0))
                                        .rounded(px(6.0))
                                        .bg(if active {
                                            palette.accent_green_soft
                                        } else {
                                            palette.bg_selected
                                        })
                                        .text_size(px(10.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(if active {
                                            palette.accent_green
                                        } else {
                                            palette.accent_blue
                                        })
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            if !was_blind_drag_consumed() {
                                                rt_act.update_config(|c| {
                                                    c.active_ocr_plugin = Some(id_activate.clone());
                                                });
                                                cx.notify();
                                            }
                                        }))
                                        .child(if active {
                                            t!("settings.ocr.enabled_badge").to_string()
                                        } else {
                                            t!("settings.ocr.activate_btn").to_string()
                                        }),
                                )
                                .child(
                                    div()
                                        .id(SharedString::from(format!("ocr-del-{idx}")))
                                        .cursor_pointer()
                                        .text_size(px(10.0))
                                        .text_color(palette.accent_red)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if !was_blind_drag_consumed() {
                                                rt_rem.clear_ocr_download_error();
                                                if let Err(err) =
                                                    rt_rem.uninstall_ocr_plugin(&id_remove)
                                                {
                                                    this.ocr_status = Some(err.to_string());
                                                }
                                                this.cached_ocr_plugins = rt_rem.list_ocr_plugins();
                                                cx.notify();
                                            }
                                        }))
                                        .child(t!("settings.ocr.uninstall_btn").to_string()),
                                ),
                        ),
                );
            }
        }

        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(section_header("识别引擎", None, palette))
            .child(engine_group)
            .child(section_header("本地离线包", None, palette))
            .child(
                soft_group(12.0, 14.0, 10.0, palette).child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(10.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .line_height(px(16.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(palette.text_primary)
                                        .child(t!("settings.ocr.import_card_title").to_string()),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .line_height(px(14.0))
                                        .text_color(palette.text_muted)
                                        .truncate()
                                        .child(t!("settings.ocr.import_card_sub").to_string()),
                                ),
                        )
                        .child(
                            div()
                                .id("ocr-import-zip-btn")
                                .cursor_pointer()
                                .flex_shrink_0()
                                .px(px(10.0))
                                .py(px(5.0))
                                .rounded(px(7.0))
                                .bg(palette.bg_elevated)
                                .hover(|s| s.bg(palette.bg_hover))
                                .text_size(px(11.0))
                                .line_height(px(14.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.text_primary)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if !was_blind_drag_consumed() {
                                        this.open_ocr_zip_picker(cx);
                                    }
                                }))
                                .child(t!("settings.ocr.import_btn").to_string()),
                        ),
                ),
            )
            .child(
                div()
                    .pt(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .size(px(6.0))
                            .rounded(px(999.0))
                            .bg(palette.accent_green),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .line_height(px(14.0))
                            .text_color(palette.text_muted)
                            .child("全程纯离线本地识别，零云端上传"),
                    ),
            )
            .into_any_element()
    }

    /// Renders About Open Source and signed update verification settings.
    fn render_about_updates(
        &self,
        cfg: &AppConfig,
        palette: &ThemePalette,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let rt_upd = Rc::clone(&self.runtime);
        let rt_chan = Rc::clone(&self.runtime);
        let rt_check = Rc::clone(&self.runtime);
        let diag = &self.cached_session_diag;
        let update_state = self.runtime.update_status();
        let writable_dir = self.cached_writable_dir;

        let env_warning = match diag.kind {
            DesktopSessionKind::LinuxWayland => Some(t!("settings.about.wayland_hint").to_string()),
            DesktopSessionKind::MacOs if !diag.accessibility_granted => {
                Some(t!("settings.about.macos_ax_warn").to_string())
            }
            _ if !writable_dir => Some(t!("settings.about.readonly_install_hint").to_string()),
            _ => None,
        };

        let (banner_bg, dot_fg, banner_text) = match &update_state {
            UpdateStatus::Available(info) => (
                palette.accent_green_soft,
                palette.accent_green,
                t!("settings.about.available_msg", version = info.version).to_string(),
            ),
            UpdateStatus::ReadyToRestart { version, .. } => (
                palette.accent_green_soft,
                palette.accent_green,
                t!("settings.about.ready_msg", version = version).to_string(),
            ),
            UpdateStatus::AppliedPendingRelaunch(_) => (
                palette.accent_green_soft,
                palette.accent_green,
                t!("settings.about.restarted_msg").to_string(),
            ),
            UpdateStatus::UpToDate => (
                palette.accent_green_soft,
                palette.accent_green,
                t!(
                    "settings.about.up_to_date",
                    version = env!("CARGO_PKG_VERSION")
                )
                .to_string(),
            ),
            UpdateStatus::Checking => (
                palette.bg_window,
                palette.accent_blue,
                t!("settings.about.checking").to_string(),
            ),
            UpdateStatus::Downloading(_) => (
                palette.bg_window,
                palette.accent_blue,
                t!("settings.about.downloading").to_string(),
            ),
            UpdateStatus::Error(msg) => (palette.bg_window, palette.accent_red, msg.clone()),
            UpdateStatus::Idle => (
                palette.accent_green_soft,
                palette.accent_green,
                env_warning.unwrap_or_else(|| t!("settings.about.idle_title").to_string()),
            ),
        };

        let banner_cta: Option<AnyElement> = match &update_state {
            UpdateStatus::Available(info) => {
                let release_info = info.clone();
                let rt_dl = Rc::clone(&self.runtime);
                Some(
                    div()
                        .id("about-download-update-btn")
                        .cursor_pointer()
                        .flex_shrink_0()
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .bg(palette.accent_green)
                        .text_size(px(10.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(gpui_kit::rgb(0xFFFFFF))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            if !was_blind_drag_consumed() {
                                rt_dl.download_update_async(release_info.clone());
                                cx.notify();
                            }
                        }))
                        .child(t!("settings.about.download_btn").to_string())
                        .into_any_element(),
                )
            }
            UpdateStatus::ReadyToRestart { .. } => {
                let rt_apply = Rc::clone(&self.runtime);
                Some(
                    div()
                        .id("about-apply-update-btn")
                        .cursor_pointer()
                        .flex_shrink_0()
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .bg(palette.accent_green)
                        .text_size(px(10.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(gpui_kit::rgb(0xFFFFFF))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            if !was_blind_drag_consumed() {
                                if let Ok(true) = rt_apply.apply_staged_update() {
                                    cx.quit();
                                }
                                cx.notify();
                            }
                        }))
                        .child(t!("settings.about.restart_install").to_string())
                        .into_any_element(),
                )
            }
            _ => None,
        };

        let is_checking = matches!(update_state, UpdateStatus::Checking);

        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            // Hero brand row
            .child(
                div()
                    .w_full()
                    .rounded(px(11.0))
                    .bg(palette.bg_window)
                    .px(px(14.0))
                    .py(px(12.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(11.0))
                            .min_w_0()
                            .flex_1()
                            .child(brand_app_icon(36.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .min_w_0()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(6.0))
                                            .child(
                                                div()
                                                    .text_size(px(14.0))
                                                    .line_height(px(18.0))
                                                    .font_weight(ui_weight_bold())
                                                    .text_color(palette.text_primary)
                                                    .child("ClipboardX"),
                                            )
                                            .child(
                                                div()
                                                    .font_family(mono_font_family())
                                                    .text_size(px(10.5))
                                                    .line_height(px(14.0))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(palette.accent_blue)
                                                    .child(format!(
                                                        "v{}",
                                                        env!("CARGO_PKG_VERSION")
                                                    )),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .line_height(px(14.0))
                                            .text_color(palette.text_muted)
                                            .truncate()
                                            .child(t!("settings.about.tagline").to_string()),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("about-check-update-btn")
                            .when(!is_checking, |el| el.cursor_pointer())
                            .flex_shrink_0()
                            .px(px(12.0))
                            .py(px(6.0))
                            .rounded(px(7.0))
                            .bg(if is_checking {
                                palette.bg_selected
                            } else {
                                palette.accent_blue
                            })
                            .text_size(px(11.0))
                            .line_height(px(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if is_checking {
                                palette.accent_blue
                            } else {
                                gpui_kit::rgb(0xFFFFFF).into()
                            })
                            .on_click(cx.listener(move |_, _, _, cx| {
                                if !is_checking && !was_blind_drag_consumed() {
                                    rt_check.check_updates_manual();
                                    cx.notify();
                                }
                            }))
                            .child(if is_checking {
                                t!("settings.ocr.connecting_btn").to_string()
                            } else {
                                t!("settings.about.check_now").to_string()
                            }),
                    ),
            )
            // Calm status strip
            .child(
                div()
                    .w_full()
                    .rounded(px(9.0))
                    .bg(banner_bg)
                    .px(px(12.0))
                    .py(px(9.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(7.0))
                            .child(
                                div()
                                    .size(px(6.0))
                                    .flex_shrink_0()
                                    .rounded(px(999.0))
                                    .bg(dot_fg),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.0))
                                    .line_height(px(15.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(dot_fg)
                                    .child(banner_text),
                            ),
                    )
                    .when_some(banner_cta, |el, btn| el.child(btn)),
            )
            // Update channel + silent switch
            .child(
                soft_group(10.0, 14.0, 10.0, palette)
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .line_height(px(16.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(palette.text_primary)
                                            .child(t!("settings.about.channel").to_string()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.0))
                                            .line_height(px(14.0))
                                            .text_color(palette.text_muted)
                                            .truncate()
                                            .child(t!("settings.about.channel_sub").to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(116.0))
                                    .flex_shrink_0()
                                    .rounded(px(8.0))
                                    .bg(palette.bg_surface)
                                    .p(px(3.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(3.0))
                                    .child(channel_pill(
                                        "chan-stable",
                                        "Stable",
                                        cfg.update_channel == UpdateChannel::Stable,
                                        palette,
                                        cx.listener({
                                            let rt = Rc::clone(&rt_chan);
                                            move |_, _, _, cx| {
                                                rt.update_config(|c| {
                                                    c.update_channel = UpdateChannel::Stable
                                                });
                                                cx.notify();
                                            }
                                        }),
                                    ))
                                    .child(channel_pill(
                                        "chan-beta",
                                        "Beta",
                                        cfg.update_channel == UpdateChannel::Beta,
                                        palette,
                                        cx.listener(move |_, _, _, cx| {
                                            rt_chan.update_config(|c| {
                                                c.update_channel = UpdateChannel::Beta
                                            });
                                            cx.notify();
                                        }),
                                    )),
                            ),
                    )
                    .child(clean_toggle_row(
                        "toggle-bg-update",
                        &t!("settings.about.bg_update"),
                        "关闭后进入 100% 纯断网离线模式",
                        cfg.background_auto_update,
                        palette,
                        cx.listener(move |_, _, _, cx| {
                            rt_upd.update_config(|c| {
                                c.background_auto_update = !c.background_auto_update
                            });
                            cx.notify();
                        }),
                    )),
            )
            // Open source footer
            .child(section_header(
                &format!(
                    "{} · {}",
                    t!("settings.about.oss_title"),
                    diag.display_label
                ),
                Some(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(10.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.accent_purple)
                        .child("MIT / Apache-2.0")
                        .into_any_element(),
                ),
                palette,
            ))
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap(px(8.0))
                    .child(external_link_btn(
                        "oss-repo-btn",
                        &t!("settings.about.repo_btn"),
                        REPO_URL,
                        palette,
                    ))
                    .child(external_link_btn(
                        "oss-issue-btn",
                        &t!("settings.about.issue_btn"),
                        ISSUES_URL,
                        palette,
                    ))
                    .child(external_link_btn(
                        "oss-notice-btn",
                        &t!("settings.about.notice_btn"),
                        NOTICES_URL,
                        palette,
                    )),
            )
            .into_any_element()
    }
}

fn section_header(label: &str, right: Option<AnyElement>, palette: &ThemePalette) -> AnyElement {
    div()
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(
            div()
                .text_size(px(10.0))
                .line_height(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.text_muted)
                .child(label.to_owned()),
        )
        .when_some(right, |el, node| el.child(node))
        .into_any_element()
}

fn soft_group(pad_y: f32, pad_x: f32, gap: f32, palette: &ThemePalette) -> gpui_kit::Div {
    div()
        .w_full()
        .rounded(px(10.0))
        .bg(palette.bg_window)
        .px(px(pad_x))
        .py(px(pad_y))
        .flex()
        .flex_col()
        .gap(px(gap))
}

fn lang_pill(
    id: &'static str,
    label: &'static str,
    active: bool,
    palette: &ThemePalette,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .flex_1()
        .h(px(22.0))
        .rounded(px(6.0))
        .bg(if active {
            palette.bg_selected
        } else {
            gpui_kit::transparent_black()
        })
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(10.5))
        .line_height(px(14.0))
        .font_weight(if active {
            FontWeight::MEDIUM
        } else {
            FontWeight::NORMAL
        })
        .text_color(if active {
            palette.accent_blue
        } else {
            palette.text_muted
        })
        .on_click(move |ev, win, cx| {
            if !was_blind_drag_consumed() {
                on_click(ev, win, cx);
            }
        })
        .child(label)
        .into_any_element()
}

fn hotkey_row(label: &str, combo: &str, palette: &ThemePalette) -> AnyElement {
    div()
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(
            div()
                .text_size(px(12.0))
                .line_height(px(16.0))
                .text_color(palette.text_primary)
                .child(label.to_owned()),
        )
        .child(
            div()
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .bg(palette.bg_elevated)
                .font_family(mono_font_family())
                .text_size(px(10.0))
                .line_height(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.text_primary)
                .child(combo.replace('+', " + ")),
        )
        .into_any_element()
}

fn clean_segment_bar(
    track_bg: gpui_kit::Hsla,
    items: Vec<(String, bool, SegmentClickHandler)>,
    palette: &ThemePalette,
) -> AnyElement {
    div()
        .w_full()
        .rounded(px(8.0))
        .bg(track_bg)
        .p(px(3.0))
        .flex()
        .items_center()
        .gap(px(3.0))
        .children(
            items
                .into_iter()
                .enumerate()
                .map(|(idx, (label, active, cb))| {
                    div()
                        .id(SharedString::from(format!("clean-seg-{idx}-{label}")))
                        .cursor_pointer()
                        .flex_1()
                        .py(px(5.0))
                        .px(px(8.0))
                        .rounded(px(6.0))
                        .bg(if active {
                            palette.bg_selected
                        } else {
                            gpui_kit::transparent_black()
                        })
                        .hover(|s| if active { s } else { s.bg(palette.bg_hover) })
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(11.0))
                        .line_height(px(14.0))
                        .font_weight(if active {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if active {
                            palette.accent_blue
                        } else {
                            palette.text_secondary
                        })
                        .on_click(move |ev, win, cx| {
                            if !was_blind_drag_consumed() {
                                cb(ev, win, cx);
                            }
                        })
                        .child(label)
                }),
        )
        .into_any_element()
}

fn channel_pill(
    id: &'static str,
    label: &'static str,
    active: bool,
    palette: &ThemePalette,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .flex_1()
        .py(px(4.0))
        .rounded(px(6.0))
        .bg(if active {
            palette.bg_selected
        } else {
            gpui_kit::transparent_black()
        })
        .hover(|s| if active { s } else { s.bg(palette.bg_hover) })
        .flex()
        .items_center()
        .justify_center()
        .font_family(mono_font_family())
        .text_size(px(10.0))
        .line_height(px(13.0))
        .font_weight(if active {
            FontWeight::MEDIUM
        } else {
            FontWeight::NORMAL
        })
        .text_color(if active {
            palette.accent_blue
        } else {
            palette.text_secondary
        })
        .on_click(move |ev, win, cx| {
            if !was_blind_drag_consumed() {
                on_click(ev, win, cx);
            }
        })
        .child(label)
        .into_any_element()
}

fn toggle_switch(
    id: &'static str,
    enabled: bool,
    palette: &ThemePalette,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .w(px(32.0))
        .h(px(18.0))
        .flex_shrink_0()
        .rounded(px(999.0))
        .bg(if enabled {
            palette.accent_blue
        } else {
            palette.bg_elevated
        })
        .p(px(2.0))
        .flex()
        .items_center()
        .justify_end()
        .when(!enabled, |el| el.justify_start())
        .on_click(move |ev, win, cx| {
            if !was_blind_drag_consumed() {
                on_click(ev, win, cx);
            }
        })
        .child(
            div()
                .size(px(14.0))
                .rounded(px(999.0))
                .bg(gpui_kit::rgb(0xFFFFFF)),
        )
        .into_any_element()
}

fn clean_toggle_row(
    id: &'static str,
    label: &str,
    sublabel: &str,
    enabled: bool,
    palette: &ThemePalette,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .w_full()
        .on_click(move |ev, win, cx| {
            if !was_blind_drag_consumed() {
                on_click(ev, win, cx);
            }
        })
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .text_size(px(12.0))
                        .line_height(px(16.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_primary)
                        .truncate()
                        .child(label.to_owned()),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .line_height(px(14.0))
                        .text_color(palette.text_muted)
                        .truncate()
                        .child(sublabel.to_owned()),
                ),
        )
        .child(
            div()
                .w(px(32.0))
                .h(px(18.0))
                .flex_shrink_0()
                .rounded(px(999.0))
                .bg(if enabled {
                    palette.accent_blue
                } else {
                    palette.bg_elevated
                })
                .p(px(2.0))
                .flex()
                .items_center()
                .justify_end()
                .when(!enabled, |el| el.justify_start())
                .child(
                    div()
                        .size(px(14.0))
                        .rounded(px(999.0))
                        .bg(gpui_kit::rgb(0xFFFFFF)),
                ),
        )
        .into_any_element()
}

#[derive(Clone, Copy)]
struct ThemeSwatch {
    bg: RgbHex,
    pane: RgbHex,
    accent: RgbHex,
}

impl ThemeSwatch {
    const fn new(bg: u32, pane: u32, accent: u32) -> Self {
        Self {
            bg: RgbHex(bg),
            pane: RgbHex(pane),
            accent: RgbHex(accent),
        }
    }
}

fn theme_preview_card(
    id: ThemeId,
    tag: &'static str,
    swatch: ThemeSwatch,
    active: bool,
    palette: &ThemePalette,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(SharedString::from(format!(
            "theme-card-{}",
            id.display_name()
        )))
        .cursor_pointer()
        .flex_1()
        .rounded(px(10.0))
        .bg(if active {
            palette.bg_selected
        } else {
            palette.bg_window
        })
        .hover(|s| if active { s } else { s.bg(palette.bg_hover) })
        .px(px(12.0))
        .py(px(10.0))
        .on_click(move |ev, win, cx| {
            if !was_blind_drag_consumed() {
                on_click(ev, win, cx);
            }
        })
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .w_full()
                .h(px(24.0))
                .rounded(px(6.0))
                .bg(hex_to_hsla(swatch.bg, 1.0))
                .px(px(6.0))
                .py(px(5.0))
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(
                    div()
                        .flex_1()
                        .h(px(12.0))
                        .rounded(px(4.0))
                        .bg(hex_to_hsla(swatch.pane, 1.0)),
                )
                .child(
                    div()
                        .w(px(16.0))
                        .h(px(12.0))
                        .rounded(px(4.0))
                        .bg(hex_to_hsla(swatch.accent, 1.0)),
                ),
        )
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(px(11.0))
                        .line_height(px(15.0))
                        .font_weight(if active {
                            ui_weight_bold()
                        } else {
                            FontWeight::MEDIUM
                        })
                        .text_color(if active {
                            palette.accent_blue
                        } else {
                            palette.text_primary
                        })
                        .child(id.display_name()),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .text_color(if active {
                            palette.accent_blue
                        } else {
                            palette.text_muted
                        })
                        .child(tag),
                ),
        )
        .into_any_element()
}

fn policy_option_card(
    id: &'static str,
    title: &str,
    subtitle: &str,
    active: bool,
    palette: &ThemePalette,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .flex_1()
        .rounded(px(9.0))
        .bg(if active {
            palette.bg_selected
        } else {
            palette.bg_window
        })
        .hover(|s| if active { s } else { s.bg(palette.bg_hover) })
        .px(px(10.0))
        .py(px(9.0))
        .on_click(move |ev, win, cx| {
            if !was_blind_drag_consumed() {
                on_click(ev, win, cx);
            }
        })
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .text_size(px(11.0))
                .line_height(px(14.0))
                .font_weight(if active {
                    ui_weight_bold()
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(if active {
                    palette.accent_blue
                } else {
                    palette.text_primary
                })
                .truncate()
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_size(px(9.5))
                .line_height(px(13.0))
                .text_color(palette.text_muted)
                .truncate()
                .child(subtitle.to_owned()),
        )
        .into_any_element()
}

fn storage_legend_dot(color: gpui_kit::Hsla, label: &str, palette: &ThemePalette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .child(div().size(px(6.0)).rounded(px(999.0)).bg(color))
        .child(
            div()
                .text_size(px(10.0))
                .line_height(px(13.0))
                .text_color(palette.text_muted)
                .child(label.to_owned()),
        )
        .into_any_element()
}

fn retention_selector_cell(
    id: &'static str,
    label: &str,
    value: &str,
    palette: &ThemePalette,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .flex_1()
        .rounded(px(9.0))
        .bg(palette.bg_window)
        .hover(|s| s.bg(palette.bg_hover))
        .px(px(12.0))
        .py(px(9.0))
        .on_click(move |ev, win, cx| {
            if !was_blind_drag_consumed() {
                on_click(ev, win, cx);
            }
        })
        .flex()
        .flex_col()
        .gap(px(3.0))
        .child(
            div()
                .text_size(px(10.0))
                .line_height(px(13.0))
                .text_color(palette.text_muted)
                .child(label.to_owned()),
        )
        .child(
            div()
                .text_size(px(13.0))
                .line_height(px(17.0))
                .font_weight(ui_weight_bold())
                .text_color(palette.text_primary)
                .child(value.to_owned()),
        )
        .into_any_element()
}

fn external_link_btn(
    id: &'static str,
    label: &str,
    url: &'static str,
    palette: &ThemePalette,
) -> AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .h(px(28.0))
        .rounded(px(8.0))
        .bg(palette.bg_window)
        .hover(|s| s.bg(palette.bg_hover))
        .px(px(10.0))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(5.0))
        .text_size(px(10.5))
        .line_height(px(14.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(palette.text_secondary)
        .on_click(move |_, _, _| {
            if !was_blind_drag_consumed() {
                let _ = open_external_url(url);
            }
        })
        .child(div().min_w_0().truncate().child(label.to_owned()))
        .child(app_icon(AppIcon::External, 10.0, palette.text_muted))
        .into_any_element()
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn format_thousands(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (idx, ch) in s.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn next_cycle<T: Copy + PartialEq>(options: &[T], current: T) -> T {
    let idx = options
        .iter()
        .position(|&v| v == current)
        .map(|i| (i + 1) % options.len())
        .unwrap_or(0);
    options[idx]
}
