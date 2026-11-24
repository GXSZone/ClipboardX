pub mod capsule;
pub mod image_preview_popup;
pub mod tray_menu;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clipboardx_model::{
    ClipItemId, HotkeyEventKind, LayoutSpec, PlatformBridge, ThemeId, WindowBlurBehavior,
};
use clipboardx_platform::hotkey::TriggerOutcome;
use clipboardx_platform::{
    TrayAction, WindowResizeEdge, configure_custom_shadow_window, configure_transient_popup_window,
    cursor_screen_y, is_cursor_over_own_window, is_left_mouse_button_down,
    own_window_visual_origin, own_window_visual_rect, place_own_window_at_capsule_anchor,
    position_hover_preview_popup, remember_own_window_hwnd, reset_hover_preview_popup_hwnd,
    resize_own_window_clamped, set_own_window_topmost, snap_or_read_capsule_anchor,
    trim_process_working_set,
};
#[cfg(target_os = "windows")]
use clipboardx_platform::{
    begin_native_window_resize, hide_hover_preview_popup, is_own_process_foreground,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext, Bounds, Context, CursorStyle, Entity, Image,
    MouseButton, Render, ResizeEdge, Task, Window, WindowBackgroundAppearance, WindowBounds,
    WindowKind, WindowOptions, div, point, px, size,
};

use crate::history::{HistoryPanel, PanelRequest};
use crate::icons::{BlindDragTracker, is_blind_dragging};
use crate::settings::SettingsView;
use crate::shell::capsule::{
    CAPSULE_IDLE_HEIGHT, CAPSULE_IDLE_WIDTH, CAPSULE_PULSE_MS, capsule_liquid_glass_style,
    render_idle_mini_capsule,
};
use crate::shell::image_preview_popup::{
    HOVER_CARD_GAP, HOVER_IMG_MAX_HEIGHT, HOVER_IMG_MAX_WIDTH, HoverImageSpec,
    ImageHoverPreviewView, compute_hover_card_geometry,
};
use crate::shell::tray_menu::{TRAY_MENU_HEIGHT, TRAY_MENU_WIDTH, TrayMenuView};
use crate::state::{AppRuntime, UiEvent};
use crate::theme::{
    MotionSpec, ThemePalette, ease_out_expo_cubic, lerp_f32, resolve_theme_id, spring_overshoot,
    sync_component_theme,
};

const DOCK_MARGIN_LOGICAL: f32 = 12.0;
const CAPSULE_DOCK_MARGIN_LOGICAL: f32 = 10.0;
const CAPSULE_SNAP_THRESHOLD_LOGICAL: f32 = 24.0;
const CAPSULE_IDLE_RADIUS: f32 = CAPSULE_IDLE_WIDTH * 0.5;
const CAPSULE_LEAVE_COLLAPSE_MS: u64 = 650;

/// Outer transparent margin reserved around the visual rounded surface on Windows so
/// anti-aliased custom shadows render cleanly without DWM non-client square shadows,
/// paired with `WM_NCHITTEST -> HTTRANSPARENT` pass-through for all exterior pixels.
#[cfg(target_os = "windows")]
pub const WINDOW_SHADOW_MARGIN: f32 = 22.0;
#[cfg(not(target_os = "windows"))]
pub const WINDOW_SHADOW_MARGIN: f32 = 0.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivePage {
    History,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FloatingStage {
    FullPanel,
    MiniCapsuleIdle,
}

/// Deferred native HWND action executed atomically when a closing `ShellMorphAnim` reaches `1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum MorphCompletion {
    None,
    /// Shrinks the native HWND to `38x58` and plays the landing settle bounce after collapsing to capsule.
    ShrinkToCapsuleIdle,
    /// Switches `HistoryPanel` to slim mode (`if apply_slim`) and shrinks the native HWND to `328px`
    /// after a zero-reflow `680 -> 328` GPU shell morph.
    ShrinkToSlimPanel {
        restore_origin: Option<(i32, i32)>,
        target_height: f32,
        apply_slim: bool,
    },
}

/// Unified zero-reflow pure-GPU morph transition (`width`, `height`, `radius`, `opacity`)
/// shared across all window transitions:
/// - Capsule (`38x58`) <-> Full slim (`328x420`) / dual-pane (`680x420`) panel (`220ms`)
/// - Slim <-> Dual-pane drawer toggle (`328x420 <-> 680x420`, `180ms`)
/// - Slim <-> Settings width transition (`328x420 <-> 680x420`, `180ms`)
#[derive(Debug, Clone, Copy)]
struct ShellMorphAnim {
    from_w: f32,
    from_h: f32,
    from_r: f32,
    to_w: f32,
    to_h: f32,
    to_r: f32,
    inner_w: f32,
    inner_h: f32,
    align_right: bool,
    fade_content: bool,
    completion: MorphCompletion,
    started_at: Instant,
    duration_ms: f32,
}

impl ShellMorphAnim {
    fn raw_progress(&self, now: Instant) -> f32 {
        (now.saturating_duration_since(self.started_at).as_secs_f32() * 1000.0 / self.duration_ms)
            .clamp(0.0, 1.0)
    }

    fn is_closing_to_idle(&self) -> bool {
        matches!(self.completion, MorphCompletion::ShrinkToCapsuleIdle)
    }

    fn sample(&self, now: Instant) -> (f32, f32, f32, f32, bool) {
        let raw = self.raw_progress(now);
        let p = ease_out_expo_cubic(raw);
        (
            lerp_f32(self.from_w, self.to_w, p),
            lerp_f32(self.from_h, self.to_h, p),
            lerp_f32(self.from_r, self.to_r, p),
            p,
            raw >= 1.0,
        )
    }
}

/// Bidirectional main-panel <-> settings page transition (`220ms`).
#[derive(Debug, Clone, Copy)]
struct PageTransition {
    to_settings: bool,
    started_at: Instant,
    duration_ms: f32,
}

impl PageTransition {
    fn raw_progress(&self, now: Instant) -> f32 {
        (now.saturating_duration_since(self.started_at).as_secs_f32() * 1000.0 / self.duration_ms)
            .clamp(0.0, 1.0)
    }

    fn eased_progress(&self, now: Instant) -> f32 {
        ease_out_expo_cubic(self.raw_progress(now))
    }
}

/// Global 280ms theme color cross-fade.
#[derive(Debug, Clone)]
struct ThemeTransition {
    from: ThemePalette,
    to: ThemePalette,
    started_at: Instant,
    duration_ms: f32,
}

enum ShellAction {
    Panel(PanelRequest),
    CapsuleExpandFull,
}

fn sync_native_shadow_region(window: &Window, visual_radius: f32) {
    let scale = window.scale_factor();
    configure_custom_shadow_window(
        (WINDOW_SHADOW_MARGIN * scale).round() as i32,
        (visual_radius * scale).round() as i32,
    );
}

fn sync_native_shadow_region_scaled(scale: f32, visual_radius: f32) {
    configure_custom_shadow_window(
        (WINDOW_SHADOW_MARGIN * scale).round() as i32,
        (visual_radius * scale).round() as i32,
    );
}

fn resize_visual_window(window: &mut Window, visual_w: f32, visual_h: f32) {
    let pad = WINDOW_SHADOW_MARGIN * 2.0;
    window.resize(size(px(visual_w + pad), px(visual_h + pad)));
}

/// Unified single floating window workspace hosting the 3 clean window forms:
/// 1. Pure-icon floating mini-capsule (`38x58`)
/// 2. Full slim single-column panel (`328x420`)
/// 3. Full dual-pane panel & Settings center (`680x420`)
///
/// All stage transitions share the unified zero-reflow VSync GPU morph architecture.
pub struct AppWorkspaceView {
    runtime: Rc<AppRuntime>,
    on_action: Rc<dyn Fn(ShellAction)>,
    page: ActivePage,
    stage: FloatingStage,
    window_pinned: bool,
    hover_expanded_panel: bool,
    capsule_anchor: Option<(i32, i32, bool)>,
    history_snapshot_dirty: bool,
    was_slim_before_settings: bool,
    pre_settings_origin: Option<(i32, i32)>,
    settings_settled_origin: Option<(i32, i32)>,
    pre_expand_origin: Option<(i32, i32)>,
    expanded_settled_origin: Option<(i32, i32)>,
    is_focused: bool,
    is_pointer_over: bool,
    hover_entered_at: Option<Instant>,
    hover_left_at: Option<Instant>,
    suppress_hover_until_leave: bool,
    history: Entity<HistoryPanel>,
    settings: Option<Entity<SettingsView>>,
    drag: BlindDragTracker,

    window_geom_task: Option<Task<()>>,
    pending_stage_transition: bool,
    locked_shell_size: Option<(f32, f32)>,
    locked_align_right: bool,
    frame_scheduled: bool,

    shell_morph: Option<ShellMorphAnim>,
    capsule_hover_pop_start: Option<Instant>,
    capsule_settle_bounce_start: Option<Instant>,
    capsule_pulse_start: Option<Instant>,

    active_theme_id: ThemeId,
    rendered_palette: ThemePalette,
    theme_transition: Option<ThemeTransition>,

    page_transition: Option<PageTransition>,
}

impl AppWorkspaceView {
    fn new(
        runtime: Rc<AppRuntime>,
        on_action: Rc<dyn Fn(ShellAction)>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let cfg = runtime.snapshot_config();
        let active_theme_id = resolve_theme_id(&cfg, window.appearance());
        let rendered_palette = ThemePalette::from_theme_id(active_theme_id);
        sync_component_theme(&rendered_palette, Some(window), cx);

        let history_rt = Rc::clone(&runtime);
        let action_for_panel = Rc::clone(&on_action);
        let history = cx.new(|cx| {
            HistoryPanel::new(
                history_rt,
                Rc::new(move |req| action_for_panel(ShellAction::Panel(req))),
                window,
                cx,
            )
        });

        Self {
            runtime,
            on_action,
            page: ActivePage::History,
            stage: FloatingStage::FullPanel,
            window_pinned: false,
            hover_expanded_panel: false,
            capsule_anchor: None,
            history_snapshot_dirty: false,
            was_slim_before_settings: false,
            pre_settings_origin: None,
            settings_settled_origin: None,
            pre_expand_origin: None,
            expanded_settled_origin: None,
            is_focused: true,
            is_pointer_over: false,
            hover_entered_at: None,
            hover_left_at: None,
            suppress_hover_until_leave: false,
            history,
            settings: None,
            drag: BlindDragTracker::default(),
            window_geom_task: None,
            pending_stage_transition: false,
            locked_shell_size: None,
            locked_align_right: true,
            frame_scheduled: false,
            shell_morph: None,
            capsule_hover_pop_start: None,
            capsule_settle_bounce_start: None,
            capsule_pulse_start: None,
            active_theme_id,
            rendered_palette,
            theme_transition: None,
            page_transition: None,
        }
    }

    pub fn is_on_settings(&self) -> bool {
        self.page == ActivePage::Settings
    }

    pub fn is_window_pinned(&self) -> bool {
        self.window_pinned
    }

    pub fn is_morphing_or_locked(&self) -> bool {
        self.shell_morph.is_some()
            || self.locked_shell_size.is_some()
            || self.pending_stage_transition
    }

    pub fn toggle_window_pin(&mut self, cx: &mut Context<'_, Self>) -> bool {
        self.window_pinned = !self.window_pinned;
        let pinned = self.window_pinned;
        if pinned {
            self.hover_expanded_panel = false;
        } else if !self.is_pointer_over {
            self.hover_left_at = Some(Instant::now());
        }
        self.history
            .update(cx, |h, cx| h.set_window_pinned(pinned, cx));
        cx.notify();
        pinned
    }

    pub fn finish_blind_drag(&mut self, window: &Window) -> bool {
        let was_dragging = self.drag.on_mouse_up();
        if was_dragging {
            self.hover_entered_at = None;
            self.hover_left_at = None;
            let scale = window.scale_factor();
            let margin_logical = if self.stage == FloatingStage::MiniCapsuleIdle {
                CAPSULE_DOCK_MARGIN_LOGICAL
            } else {
                DOCK_MARGIN_LOGICAL
            };
            let phys_margin = (margin_logical * scale).round() as i32;
            let snap_phys = (CAPSULE_SNAP_THRESHOLD_LOGICAL * scale).round() as i32;
            if let Some(anchor) = snap_or_read_capsule_anchor(phys_margin, snap_phys) {
                self.capsule_anchor = Some(anchor);
            }
        }
        was_dragging
    }

    fn capsule_anchor_is_right(&self) -> bool {
        self.capsule_anchor
            .map(|(_, _, right)| right)
            .unwrap_or(true)
    }

    fn has_active_animations(&self) -> bool {
        self.shell_morph.is_some()
            || self.theme_transition.is_some()
            || self.page_transition.is_some()
            || self.capsule_hover_pop_start.is_some()
            || self.capsule_settle_bounce_start.is_some()
            || self.capsule_pulse_start.is_some()
    }

    /// Schedules a VSync-aligned `window.on_next_frame` callback to step active animations.
    /// Safe to invoke from any lifecycle phase (event handlers, async tasks, next-frame
    /// callbacks, or `Render::render`): `on_next_frame` schedules and wakes the platform frame
    /// source directly without touching `current_view()`.
    fn schedule_animation_frame(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.frame_scheduled || !self.has_active_animations() {
            return;
        }
        self.frame_scheduled = true;
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            let _ = weak.update(cx, |ws, cx| {
                ws.frame_scheduled = false;
                ws.step_animations(window, cx);
            });
        });
    }

    /// Advances all active VSync shell animations, propagates interpolated theme state
    /// safely outside `render`, and executes completion transitions atomically.
    fn step_animations(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let now = Instant::now();

        // 1. 280ms global theme palette cross-fade.
        if let Some(tr) = &self.theme_transition {
            let raw_t = (now.saturating_duration_since(tr.started_at).as_secs_f32() * 1000.0
                / tr.duration_ms)
                .clamp(0.0, 1.0);
            let eased = ease_out_expo_cubic(raw_t);
            let current = ThemePalette::lerp(&tr.from, &tr.to, eased);
            self.rendered_palette = current.clone();
            self.history
                .update(cx, |h, cx| h.set_animated_palette(current.clone(), cx));
            if let Some(settings) = &self.settings {
                settings.update(cx, |s, cx| s.set_animated_palette(current, cx));
            }
            if raw_t >= 1.0 {
                self.theme_transition = None;
            }
        }

        // 2. Unified zero-reflow GPU shell morph.
        if let Some(morph) = self.shell_morph
            && morph.raw_progress(now) >= 1.0
        {
            self.shell_morph = None;
            match morph.completion {
                MorphCompletion::None => {}
                MorphCompletion::ShrinkToCapsuleIdle => {
                    self.stage = FloatingStage::MiniCapsuleIdle;
                    self.hover_expanded_panel = false;
                    self.is_focused = false;
                    self.locked_shell_size = Some((CAPSULE_IDLE_WIDTH, CAPSULE_IDLE_HEIGHT));
                    self.locked_align_right = self.capsule_anchor_is_right();
                    self.capsule_hover_pop_start = None;
                    self.capsule_settle_bounce_start = Some(now);
                    if self.page == ActivePage::Settings {
                        self.page = ActivePage::History;
                    }
                    let scale = window.scale_factor();
                    let win_handle = window.window_handle();
                    let phys_w = (CAPSULE_IDLE_WIDTH * scale).round() as i32;
                    let phys_h = (CAPSULE_IDLE_HEIGHT * scale).round() as i32;
                    let phys_margin = (CAPSULE_DOCK_MARGIN_LOGICAL * scale).round() as i32;
                    let anchor = self.capsule_anchor;
                    self.window_geom_task = Some(cx.spawn(async move |this, cx| {
                        sync_native_shadow_region_scaled(scale, CAPSULE_IDLE_RADIUS);
                        let resolved =
                            place_own_window_at_capsule_anchor(phys_w, phys_h, phys_margin, anchor);
                        if resolved.is_none() {
                            let _ = win_handle.update(cx, |_, window, _| {
                                resize_visual_window(
                                    window,
                                    CAPSULE_IDLE_WIDTH,
                                    CAPSULE_IDLE_HEIGHT,
                                );
                            });
                        }
                        let _ = win_handle.update(cx, |_, window, cx| {
                            if let Some(a) = resolved {
                                let _ = this.update(cx, |ws, _| {
                                    ws.capsule_anchor = Some(a);
                                });
                            }
                            window.refresh();
                            let weak = this.clone();
                            window.on_next_frame(move |window, cx| {
                                let _ = weak.update(cx, |ws, cx| {
                                    ws.locked_shell_size = None;
                                    ws.schedule_animation_frame(window, cx);
                                    cx.notify();
                                });
                            });
                        });
                    }));
                }
                MorphCompletion::ShrinkToSlimPanel {
                    restore_origin,
                    target_height,
                    apply_slim,
                } => {
                    self.locked_shell_size = Some((LayoutSpec::WINDOW_SLIM_WIDTH, target_height));
                    self.locked_align_right = morph.align_right;
                    if apply_slim {
                        self.history.update(cx, |h, cx| {
                            h.apply_slim_mode(true, window, cx);
                        });
                    }
                    let scale = window.scale_factor();
                    let win_handle = window.window_handle();
                    let phys_w = (LayoutSpec::WINDOW_SLIM_WIDTH * scale).round() as i32;
                    let phys_h = (target_height * scale).round() as i32;
                    let edge_margin_phys = (DOCK_MARGIN_LOGICAL * scale).round() as i32;
                    self.window_geom_task = Some(cx.spawn(async move |this, cx| {
                        sync_native_shadow_region_scaled(scale, LayoutSpec::WINDOW_RADIUS);
                        let settled = resize_own_window_clamped(
                            phys_w,
                            phys_h,
                            restore_origin,
                            edge_margin_phys,
                        );
                        if settled.is_none() {
                            let _ = win_handle.update(cx, |_, window, _| {
                                resize_visual_window(
                                    window,
                                    LayoutSpec::WINDOW_SLIM_WIDTH,
                                    target_height,
                                );
                            });
                        }
                        let _ = win_handle.update(cx, |_, window, _| {
                            window.refresh();
                            let weak = this.clone();
                            window.on_next_frame(move |window, cx| {
                                let _ = weak.update(cx, |ws, cx| {
                                    ws.locked_shell_size = None;
                                    ws.schedule_animation_frame(window, cx);
                                    cx.notify();
                                });
                            });
                        });
                    }));
                }
            }
        }

        // 3. Capsule micro animations (hover spring, landing bounce, capture pulse).
        if let Some(s) = self.capsule_hover_pop_start
            && now.saturating_duration_since(s).as_secs_f32() * 1000.0
                >= MotionSpec::CAPSULE_HOVER_SPRING_MS
        {
            self.capsule_hover_pop_start = None;
        }
        if let Some(s) = self.capsule_settle_bounce_start
            && now.saturating_duration_since(s).as_secs_f32() * 1000.0
                >= MotionSpec::CAPSULE_SETTLE_TOTAL_MS
        {
            self.capsule_settle_bounce_start = None;
        }
        if let Some(s) = self.capsule_pulse_start
            && now.saturating_duration_since(s).as_secs_f32() * 1000.0 >= CAPSULE_PULSE_MS
        {
            self.capsule_pulse_start = None;
        }

        // 4. 220ms main-panel <-> settings transition.
        if let Some(tr) = self.page_transition
            && tr.raw_progress(now) >= 1.0
        {
            self.page_transition = None;
            if !tr.to_settings && self.page == ActivePage::History {
                self.settings = None;
            }
        }

        if self.has_active_animations() {
            self.schedule_animation_frame(window, cx);
        }
        cx.notify();
    }

    fn schedule_clamped_resize(
        &mut self,
        visual_w: f32,
        visual_h: f32,
        preferred_origin: Option<(i32, i32)>,
        record_settings_origin: bool,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let scale = window.scale_factor();
        let win_handle = window.window_handle();
        let phys_w = (visual_w * scale).round() as i32;
        let phys_h = (visual_h * scale).round() as i32;
        let phys_margin = (DOCK_MARGIN_LOGICAL * scale).round() as i32;
        self.window_geom_task = Some(cx.spawn(async move |this, cx| {
            sync_native_shadow_region_scaled(scale, LayoutSpec::WINDOW_RADIUS);
            let settled = resize_own_window_clamped(phys_w, phys_h, preferred_origin, phys_margin);
            if settled.is_none() {
                let _ = win_handle.update(cx, |_, window, _| {
                    resize_visual_window(window, visual_w, visual_h);
                });
            }
            if record_settings_origin {
                let _ = this.update(cx, |ws, _| {
                    ws.settings_settled_origin = settled;
                });
            }
        }));
    }

    /// Performs a zero-reflow pure-GPU `ShellMorphAnim` drawer toggle between `328px` slim and
    /// `680px` dual-pane modes (`180ms`), using the zero-reflow
    /// GPU clipping architecture of `Alt+V` (`restore_full_panel`).
    pub fn toggle_compact_animated(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.page != ActivePage::History || self.stage != FloatingStage::FullPanel {
            return;
        }
        let target_slim = !self.history.read(cx).is_slim();
        let cfg = self.runtime.snapshot_config();
        let cur_h = self.current_visual_height(window);
        let scale = window.scale_factor();
        let win_handle = window.window_handle();
        let edge_margin_phys = (DOCK_MARGIN_LOGICAL * scale).round() as i32;

        self.shell_morph = None;
        self.locked_shell_size = None;

        if !target_slim {
            // Slim (328) -> Dual (680): lock shell at 328px before HWND resize, switch HistoryPanel
            // once to 680px dual layout, and morph the outer GPU clipping shell from 328 -> 680.
            let pre_origin = own_window_visual_origin();
            self.pre_expand_origin = pre_origin;
            self.expanded_settled_origin = None;
            let phys_w = (LayoutSpec::WINDOW_DUAL_WIDTH * scale).round() as i32;
            let phys_h = (cur_h * scale).round() as i32;
            let animate = cfg.smooth_collapse_animation;

            self.locked_shell_size = Some((LayoutSpec::WINDOW_SLIM_WIDTH, cur_h));
            self.locked_align_right = false;
            cx.notify();

            self.window_geom_task = Some(cx.spawn(async move |this, cx| {
                sync_native_shadow_region_scaled(scale, LayoutSpec::WINDOW_RADIUS);
                let settled =
                    resize_own_window_clamped(phys_w, phys_h, pre_origin, edge_margin_phys);
                if settled.is_none() {
                    let _ = win_handle.update(cx, |_, window, _| {
                        resize_visual_window(window, LayoutSpec::WINDOW_DUAL_WIDTH, cur_h);
                    });
                }

                let align_right = match (pre_origin, settled) {
                    (Some((px_x, _)), Some((sx, _))) => px_x - sx > 12,
                    _ => false,
                };

                let _ = win_handle.update(cx, |_, window, cx| {
                    let _ = this.update(cx, |ws, cx| {
                        ws.locked_shell_size = None;
                        ws.expanded_settled_origin = settled;
                        ws.history.update(cx, |h, cx| {
                            h.apply_slim_mode(false, window, cx);
                        });
                        if animate {
                            ws.shell_morph = Some(ShellMorphAnim {
                                from_w: LayoutSpec::WINDOW_SLIM_WIDTH,
                                from_h: cur_h,
                                from_r: LayoutSpec::WINDOW_RADIUS,
                                to_w: LayoutSpec::WINDOW_DUAL_WIDTH,
                                to_h: cur_h,
                                to_r: LayoutSpec::WINDOW_RADIUS,
                                inner_w: LayoutSpec::WINDOW_DUAL_WIDTH,
                                inner_h: cur_h,
                                align_right,
                                fade_content: false,
                                completion: MorphCompletion::None,
                                started_at: Instant::now(),
                                duration_ms: MotionSpec::DRAWER_EXPAND_MS,
                            });
                            ws.schedule_animation_frame(window, cx);
                        }
                        cx.notify();
                    });
                });
            }));
        } else {
            // Dual (680) -> Slim (328): keep HistoryPanel static at 680px with zero per-frame
            // updates while the outer GPU shell morphs 680 -> 328 over 180ms, then shrink HWND.
            let unchanged = match (own_window_visual_origin(), self.expanded_settled_origin) {
                (Some((cx_pos, cy_pos)), Some((sx, sy))) => {
                    (cx_pos - sx).abs() <= 4 && (cy_pos - sy).abs() <= 4
                }
                _ => false,
            };
            let restore_origin = if unchanged {
                self.pre_expand_origin
            } else {
                None
            };
            let align_right = match (restore_origin, self.expanded_settled_origin) {
                (Some((rx, _)), Some((sx, _))) => rx - sx > 12,
                _ => false,
            };
            self.pre_expand_origin = None;
            self.expanded_settled_origin = None;

            if !cfg.smooth_collapse_animation {
                self.history.update(cx, |h, cx| {
                    h.apply_slim_mode(true, window, cx);
                });
                self.schedule_clamped_resize(
                    LayoutSpec::WINDOW_SLIM_WIDTH,
                    cur_h,
                    restore_origin,
                    false,
                    window,
                    cx,
                );
                return;
            }

            self.shell_morph = Some(ShellMorphAnim {
                from_w: LayoutSpec::WINDOW_DUAL_WIDTH,
                from_h: cur_h,
                from_r: LayoutSpec::WINDOW_RADIUS,
                to_w: LayoutSpec::WINDOW_SLIM_WIDTH,
                to_h: cur_h,
                to_r: LayoutSpec::WINDOW_RADIUS,
                inner_w: LayoutSpec::WINDOW_DUAL_WIDTH,
                inner_h: cur_h,
                align_right,
                fade_content: false,
                completion: MorphCompletion::ShrinkToSlimPanel {
                    restore_origin,
                    target_height: cur_h,
                    apply_slim: true,
                },
                started_at: Instant::now(),
                duration_ms: MotionSpec::DRAWER_COLLAPSE_MS,
            });
            self.schedule_animation_frame(window, cx);
            cx.notify();
        }
    }

    fn start_page_transition(&mut self, to_settings: bool, cx: &mut Context<'_, Self>) {
        let duration_ms = if to_settings {
            MotionSpec::SETTINGS_ENTER_MS
        } else {
            MotionSpec::SETTINGS_EXIT_MS
        };
        self.page_transition = Some(PageTransition {
            to_settings,
            started_at: Instant::now(),
            duration_ms,
        });
        cx.notify();
    }

    /// Triggers a `280ms` VSync-driven global `ThemePalette::lerp` cross-fade.
    fn sync_theme_transition(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let cfg = self.runtime.snapshot_config();
        let next_id = resolve_theme_id(&cfg, window.appearance());
        if next_id == self.active_theme_id {
            return;
        }
        self.active_theme_id = next_id;
        let from = self.rendered_palette.clone();
        let to = ThemePalette::from_theme_id(next_id);
        sync_component_theme(&to, Some(window), cx);

        self.theme_transition = Some(ThemeTransition {
            from,
            to,
            started_at: Instant::now(),
            duration_ms: MotionSpec::THEME_CROSSFADE_MS,
        });
        self.schedule_animation_frame(window, cx);
        cx.notify();
    }

    fn current_visual_height(&self, window: &Window) -> f32 {
        let raw_h = f32::from(window.viewport_size().height) - WINDOW_SHADOW_MARGIN * 2.0;
        raw_h.max(LayoutSpec::WINDOW_HEIGHT)
    }

    fn ensure_settings_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Entity<SettingsView> {
        if let Some(existing) = &self.settings {
            return existing.clone();
        }
        let weak_close = cx.weak_entity();
        let settings_rt = Rc::clone(&self.runtime);
        let created = cx.new(|cx| {
            SettingsView::new(
                settings_rt,
                Rc::new(move |win, app| {
                    let _ = weak_close.update(app, |ws, cx| ws.exit_settings(win, cx));
                }),
                window,
                cx,
            )
        });
        self.settings = Some(created.clone());
        created
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.page == ActivePage::Settings {
            return;
        }
        let slim_now = self.history.read(cx).is_slim();
        self.was_slim_before_settings = slim_now;
        self.is_focused = true;
        self.stage = FloatingStage::FullPanel;
        self.pending_stage_transition = false;
        self.locked_shell_size = None;
        self.shell_morph = None;

        let cur_h = self.current_visual_height(window);
        let cur_visual_w = f32::from(window.viewport_size().width) - WINDOW_SHADOW_MARGIN * 2.0;
        let rendered_palette = self.rendered_palette.clone();
        let settings_view = self.ensure_settings_view(window, cx);
        settings_view.update(cx, |s, cx| {
            s.refresh_active_tab_metrics();
            s.set_animated_palette(rendered_palette, cx);
            s.focus_self(window, cx);
        });

        if slim_now || cur_visual_w < LayoutSpec::WINDOW_DUAL_WIDTH {
            let origin = own_window_visual_origin();
            self.pre_settings_origin = origin;
            self.settings_settled_origin = None;
            let scale = window.scale_factor();
            let win_handle = window.window_handle();
            let phys_w = (LayoutSpec::WINDOW_DUAL_WIDTH * scale).round() as i32;
            let phys_h = (cur_h * scale).round() as i32;
            let phys_margin = (DOCK_MARGIN_LOGICAL * scale).round() as i32;

            self.locked_shell_size = Some((LayoutSpec::WINDOW_SLIM_WIDTH, cur_h));
            self.locked_align_right = false;
            self.page = ActivePage::Settings;
            cx.notify();

            self.window_geom_task = Some(cx.spawn(async move |this, cx| {
                sync_native_shadow_region_scaled(scale, LayoutSpec::WINDOW_RADIUS);
                let settled = resize_own_window_clamped(phys_w, phys_h, origin, phys_margin);
                if settled.is_none() {
                    let _ = win_handle.update(cx, |_, window, _| {
                        resize_visual_window(window, LayoutSpec::WINDOW_DUAL_WIDTH, cur_h);
                    });
                }
                let align_right = match (origin, settled) {
                    (Some((px_x, _)), Some((sx, _))) => px_x - sx > 12,
                    _ => false,
                };
                let _ = win_handle.update(cx, |_, window, cx| {
                    let _ = this.update(cx, |ws, cx| {
                        ws.locked_shell_size = None;
                        ws.settings_settled_origin = settled;
                        ws.shell_morph = Some(ShellMorphAnim {
                            from_w: LayoutSpec::WINDOW_SLIM_WIDTH,
                            from_h: cur_h,
                            from_r: LayoutSpec::WINDOW_RADIUS,
                            to_w: LayoutSpec::WINDOW_DUAL_WIDTH,
                            to_h: cur_h,
                            to_r: LayoutSpec::WINDOW_RADIUS,
                            inner_w: LayoutSpec::WINDOW_DUAL_WIDTH,
                            inner_h: cur_h,
                            align_right,
                            fade_content: true,
                            completion: MorphCompletion::None,
                            started_at: Instant::now(),
                            duration_ms: MotionSpec::DRAWER_EXPAND_MS,
                        });
                        ws.schedule_animation_frame(window, cx);
                        cx.notify();
                    });
                });
            }));
        } else {
            self.pre_settings_origin = None;
            self.settings_settled_origin = None;
            sync_native_shadow_region(window, LayoutSpec::WINDOW_RADIUS);
            self.page = ActivePage::Settings;
            self.start_page_transition(true, cx);
            cx.notify();
        }
    }

    pub fn exit_settings(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.page == ActivePage::History {
            return;
        }
        self.page = ActivePage::History;
        self.is_focused = true;
        self.stage = FloatingStage::FullPanel;

        let cur_h = self.current_visual_height(window);
        let slim_restore = self.history.read(cx).is_slim();
        let need_refresh = self.history_snapshot_dirty;
        self.history_snapshot_dirty = false;
        let palette = self.rendered_palette.clone();
        self.history.update(cx, |h, cx| {
            h.set_animated_palette(palette, cx);
            if need_refresh {
                h.refresh_items(cx);
            }
            h.focus_search(window, cx);
        });

        if self.was_slim_before_settings || slim_restore {
            let unchanged_pos = match (own_window_visual_origin(), self.settings_settled_origin) {
                (Some((cx_pos, cy_pos)), Some((sx, sy))) => {
                    (cx_pos - sx).abs() <= 4 && (cy_pos - sy).abs() <= 4
                }
                _ => false,
            };
            let restore_origin = if unchanged_pos {
                self.pre_settings_origin
            } else {
                None
            };
            let align_right = match (restore_origin, self.settings_settled_origin) {
                (Some((rx, _)), Some((sx, _))) => rx - sx > 12,
                _ => false,
            };
            self.was_slim_before_settings = false;
            self.pre_settings_origin = None;
            self.settings_settled_origin = None;

            self.shell_morph = Some(ShellMorphAnim {
                from_w: LayoutSpec::WINDOW_DUAL_WIDTH,
                from_h: cur_h,
                from_r: LayoutSpec::WINDOW_RADIUS,
                to_w: LayoutSpec::WINDOW_SLIM_WIDTH,
                to_h: cur_h,
                to_r: LayoutSpec::WINDOW_RADIUS,
                inner_w: LayoutSpec::WINDOW_SLIM_WIDTH,
                inner_h: cur_h,
                align_right,
                fade_content: false,
                completion: MorphCompletion::ShrinkToSlimPanel {
                    restore_origin,
                    target_height: cur_h,
                    apply_slim: false,
                },
                started_at: Instant::now(),
                duration_ms: MotionSpec::DRAWER_COLLAPSE_MS,
            });
            self.schedule_animation_frame(window, cx);
            cx.notify();
        } else {
            self.was_slim_before_settings = false;
            self.pre_settings_origin = None;
            self.settings_settled_origin = None;
            sync_native_shadow_region(window, LayoutSpec::WINDOW_RADIUS);
            self.start_page_transition(false, cx);
            cx.notify();
        }
    }

    /// Smoothly morphs the active floating panel (`328x420` slim or `680x420` dual) directly
    /// into the `38x58` vertical oval mini-capsule (`220ms` zero-reflow GPU morph + landing bounce).
    pub fn enter_mini_capsule_idle(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.stage == FloatingStage::MiniCapsuleIdle {
            return;
        }
        if self.shell_morph.is_some_and(|m| m.is_closing_to_idle()) {
            return;
        }
        self.is_focused = false;
        self.hover_entered_at = None;
        self.hover_left_at = None;
        self.suppress_hover_until_leave = true;
        self.capsule_hover_pop_start = None;
        self.pending_stage_transition = false;
        self.locked_shell_size = None;
        self.page_transition = None;

        let now = Instant::now();
        let cur_h = self.current_visual_height(window);
        let cur_w = if self.page == ActivePage::Settings || !self.history.read(cx).is_slim() {
            LayoutSpec::WINDOW_DUAL_WIDTH
        } else {
            LayoutSpec::WINDOW_SLIM_WIDTH
        };
        let (from_w, from_h, from_r) = if let Some(m) = self.shell_morph {
            let (cw, ch, cr, _, _) = m.sample(now);
            (cw, ch, cr)
        } else {
            (cur_w, cur_h, LayoutSpec::WINDOW_RADIUS)
        };

        self.shell_morph = Some(ShellMorphAnim {
            from_w,
            from_h,
            from_r,
            to_w: CAPSULE_IDLE_WIDTH,
            to_h: CAPSULE_IDLE_HEIGHT,
            to_r: CAPSULE_IDLE_RADIUS,
            inner_w: cur_w,
            inner_h: cur_h,
            align_right: self.capsule_anchor_is_right(),
            fade_content: true,
            completion: MorphCompletion::ShrinkToCapsuleIdle,
            started_at: now,
            duration_ms: MotionSpec::CAPSULE_COLLAPSE_MS,
        });
        self.schedule_animation_frame(window, cx);
        cx.notify();
    }

    /// Returns `true` when resting peacefully in mini-capsule standby with no active animations.
    pub fn is_quiet_capsule_idle(&self) -> bool {
        self.stage == FloatingStage::MiniCapsuleIdle
            && !self.is_pointer_over
            && self.shell_morph.is_none()
            && self.capsule_hover_pop_start.is_none()
            && self.capsule_pulse_start.is_none()
            && self.page_transition.is_none()
    }

    /// Drops transient preview caches and lazy views held while resting in capsule/tray standby.
    pub fn reclaim_idle_caches(&mut self, cx: &mut Context<'_, Self>) -> Option<Arc<Image>> {
        if self.page == ActivePage::History {
            self.settings = None;
        }
        self.history.update(cx, |h, _| h.reclaim_idle_caches())
    }

    /// Expands cleanly from the `38x58` mini-capsule directly into the full slim (`328x420`)
    /// or dual-pane (`680x420`) floating panel using the unified zero-reflow `Alt+V` GPU morph.
    /// When `from_hover` is `true`, keeps the panel hover-managed (auto-collapses after leaving
    /// unless focused, edited, or pinned).
    pub fn restore_full_panel(
        &mut self,
        allow_expand: bool,
        from_hover: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let now = Instant::now();
        if !from_hover {
            self.is_focused = true;
            self.hover_expanded_panel = false;
        }
        self.hover_entered_at = None;
        self.hover_left_at = None;
        self.capsule_hover_pop_start = None;
        self.capsule_settle_bounce_start = None;

        // If currently morphing back down to the mini-capsule, reverse seamlessly only when
        // expansion is actually allowed (e.g. explicit hotkey / hover, never stale OS focus).
        if allow_expand
            && self.stage == FloatingStage::FullPanel
            && let Some(m) = self.shell_morph
            && m.is_closing_to_idle()
        {
            self.suppress_hover_until_leave = false;
            let (cw, ch, cr, _, _) = m.sample(now);
            let target_w = if self.page == ActivePage::Settings || !self.history.read(cx).is_slim()
            {
                LayoutSpec::WINDOW_DUAL_WIDTH
            } else {
                LayoutSpec::WINDOW_SLIM_WIDTH
            };
            let target_h = LayoutSpec::WINDOW_HEIGHT;
            if from_hover {
                self.hover_expanded_panel = true;
            }
            self.shell_morph = Some(ShellMorphAnim {
                from_w: cw,
                from_h: ch,
                from_r: cr,
                to_w: target_w,
                to_h: target_h,
                to_r: LayoutSpec::WINDOW_RADIUS,
                inner_w: target_w,
                inner_h: target_h,
                align_right: self.capsule_anchor_is_right(),
                fade_content: true,
                completion: MorphCompletion::None,
                started_at: now,
                duration_ms: MotionSpec::FULL_EXPAND_MS,
            });
            self.schedule_animation_frame(window, cx);
            cx.notify();
            return;
        }

        let was_standby = self.stage != FloatingStage::FullPanel;
        if !was_standby || !allow_expand || self.pending_stage_transition {
            cx.notify();
            return;
        }

        self.shell_morph = None;

        if self.page != ActivePage::History {
            self.pending_stage_transition = false;
            self.locked_shell_size = None;
            self.stage = FloatingStage::FullPanel;
            cx.notify();
            return;
        }

        let (start_w, start_h, start_r) =
            (CAPSULE_IDLE_WIDTH, CAPSULE_IDLE_HEIGHT, CAPSULE_IDLE_RADIUS);

        self.pending_stage_transition = true;
        self.hover_expanded_panel = from_hover;
        self.locked_shell_size = Some((start_w, start_h));
        self.locked_align_right = self.capsule_anchor_is_right();
        cx.notify();

        let prefer_slim = self.history.read(cx).is_slim();
        let target_w = if prefer_slim {
            LayoutSpec::WINDOW_SLIM_WIDTH
        } else {
            LayoutSpec::WINDOW_DUAL_WIDTH
        };
        let target_h = LayoutSpec::WINDOW_HEIGHT;
        let scale = window.scale_factor();
        let win_handle = window.window_handle();
        let phys_w = (target_w * scale).round() as i32;
        let phys_h = (target_h * scale).round() as i32;
        let phys_margin = (DOCK_MARGIN_LOGICAL * scale).round() as i32;
        let anchor = self.capsule_anchor;

        self.window_geom_task = Some(cx.spawn(async move |this, cx| {
            sync_native_shadow_region_scaled(scale, LayoutSpec::WINDOW_RADIUS);
            let placed = if anchor.is_some() {
                place_own_window_at_capsule_anchor(phys_w, phys_h, phys_margin, anchor).is_some()
            } else {
                resize_own_window_clamped(phys_w, phys_h, None, phys_margin).is_some()
            };
            if !placed {
                let _ = win_handle.update(cx, |_, window, _| {
                    resize_visual_window(window, target_w, target_h);
                });
            }

            let _ = win_handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |ws, cx| {
                    ws.pending_stage_transition = false;
                    ws.locked_shell_size = None;
                    ws.stage = FloatingStage::FullPanel;
                    let need_refresh = ws.history_snapshot_dirty;
                    ws.history_snapshot_dirty = false;
                    ws.history.update(cx, |h, cx| {
                        if need_refresh {
                            h.refresh_items(cx);
                        }
                        if !from_hover {
                            h.focus_search(window, cx);
                        }
                    });
                    ws.shell_morph = Some(ShellMorphAnim {
                        from_w: start_w,
                        from_h: start_h,
                        from_r: start_r,
                        to_w: target_w,
                        to_h: target_h,
                        to_r: LayoutSpec::WINDOW_RADIUS,
                        inner_w: target_w,
                        inner_h: target_h,
                        align_right: ws.capsule_anchor_is_right(),
                        fade_content: true,
                        completion: MorphCompletion::None,
                        started_at: Instant::now(),
                        duration_ms: MotionSpec::FULL_EXPAND_MS,
                    });
                    ws.schedule_animation_frame(window, cx);
                    cx.notify();
                });
            });
        }));
    }

    pub fn tick_capsule_hover(
        &mut self,
        now: Instant,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if is_blind_dragging() {
            return false;
        }
        let mut left_window = false;
        if let Some(over) = is_cursor_over_own_window()
            && self.is_pointer_over != over
        {
            self.is_pointer_over = over;
            if over {
                if !self.suppress_hover_until_leave {
                    self.hover_entered_at = Some(now);
                    self.hover_left_at = None;
                    if self.stage == FloatingStage::MiniCapsuleIdle
                        && !self.pending_stage_transition
                    {
                        self.capsule_hover_pop_start = Some(now);
                        self.capsule_settle_bounce_start = None;
                        self.schedule_animation_frame(window, cx);
                    } else if self.stage == FloatingStage::FullPanel
                        && self.shell_morph.is_some_and(|m| m.is_closing_to_idle())
                    {
                        self.restore_full_panel(true, true, window, cx);
                    }
                }
            } else {
                left_window = true;
                self.suppress_hover_until_leave = false;
                self.hover_left_at = Some(now);
                self.hover_entered_at = None;
                if self.stage == FloatingStage::MiniCapsuleIdle {
                    self.capsule_hover_pop_start = None;
                }
            }
            cx.notify();
        }

        if self.pending_stage_transition {
            return left_window;
        }

        match self.stage {
            FloatingStage::MiniCapsuleIdle => {
                if self.is_pointer_over
                    && !self.suppress_hover_until_leave
                    && let Some(entered) = self.hover_entered_at
                    && now.duration_since(entered)
                        >= Duration::from_millis(MotionSpec::CAPSULE_HOVER_OPEN_MS)
                {
                    self.restore_full_panel(true, true, window, cx);
                }
            }
            FloatingStage::FullPanel => {
                let already_closing = self.shell_morph.is_some_and(|m| m.is_closing_to_idle());
                let editing = self.history.read(cx).is_editing_inline();
                if self.hover_expanded_panel
                    && !self.window_pinned
                    && !self.is_focused
                    && !self.is_on_settings()
                    && !editing
                    && !already_closing
                    && !self.is_pointer_over
                    && let Some(left) = self.hover_left_at
                    && now.duration_since(left) >= Duration::from_millis(CAPSULE_LEAVE_COLLAPSE_MS)
                {
                    self.enter_mini_capsule_idle(window, cx);
                }
            }
        }
        left_window
    }

    pub fn refresh_events(
        &mut self,
        history_changed: bool,
        config_changed: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if config_changed {
            self.sync_theme_transition(window, cx);
            self.history.update(cx, |view, cx| {
                view.refresh_locale_placeholders(window, cx);
            });
            if let Some(settings) = &self.settings {
                settings.update(cx, |view, cx| {
                    view.refresh_locale_placeholders(window, cx);
                });
            }
        }

        if history_changed {
            if self.stage == FloatingStage::MiniCapsuleIdle {
                self.history_snapshot_dirty = true;
                self.capsule_pulse_start = Some(Instant::now());
                self.schedule_animation_frame(window, cx);
            } else if self.page == ActivePage::Settings {
                self.history_snapshot_dirty = true;
                if let Some(settings) = &self.settings {
                    settings.update(cx, |view, cx| {
                        view.refresh_active_tab_metrics();
                        cx.notify();
                    });
                }
            } else {
                self.history_snapshot_dirty = false;
                self.history.update(cx, |view, cx| {
                    view.refresh_items(cx);
                });
            }
        } else if self.page == ActivePage::Settings
            && let Some(settings) = &self.settings
        {
            settings.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }
}

impl Render for AppWorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let now = Instant::now();
        let cfg = self.runtime.snapshot_config();
        let resolved = resolve_theme_id(&cfg, window.appearance());
        if resolved != self.active_theme_id && self.theme_transition.is_none() {
            self.sync_theme_transition(window, cx);
        }

        self.schedule_animation_frame(window, cx);

        let palette = self.rendered_palette.clone();
        let is_capsule_mode = self.stage == FloatingStage::MiniCapsuleIdle;

        let morph_sample = self.shell_morph.map(|m| (m, m.sample(now)));
        let locked_size = self.locked_shell_size;

        let capsule_scale = if self.stage == FloatingStage::MiniCapsuleIdle {
            if let Some(pop_start) = self.capsule_hover_pop_start {
                let t = (now.saturating_duration_since(pop_start).as_secs_f32() * 1000.0
                    / MotionSpec::CAPSULE_HOVER_SPRING_MS)
                    .clamp(0.0, 1.0);
                1.0 + (MotionSpec::CAPSULE_HOVER_ICON_SCALE - 1.0) * spring_overshoot(t)
            } else if let Some(bounce_start) = self.capsule_settle_bounce_start {
                let ms = now.saturating_duration_since(bounce_start).as_secs_f32() * 1000.0;
                if ms < MotionSpec::CAPSULE_SETTLE_DOWN_MS {
                    let p = ease_out_expo_cubic(
                        (ms / MotionSpec::CAPSULE_SETTLE_DOWN_MS).clamp(0.0, 1.0),
                    );
                    lerp_f32(1.0, MotionSpec::CAPSULE_SETTLE_MIN_SCALE, p)
                } else {
                    let p = ease_out_expo_cubic(
                        ((ms - MotionSpec::CAPSULE_SETTLE_DOWN_MS)
                            / MotionSpec::CAPSULE_SETTLE_UP_MS)
                            .clamp(0.0, 1.0),
                    );
                    lerp_f32(MotionSpec::CAPSULE_SETTLE_MIN_SCALE, 1.0, p)
                }
            } else {
                1.0
            }
        } else {
            1.0
        };

        let morph_content_opacity = match morph_sample {
            Some((m, (_, _, _, p, _))) if m.fade_content => {
                if m.is_closing_to_idle() {
                    (1.0 - p * 1.35).clamp(0.0, 1.0)
                } else {
                    ((p - 0.08) / 0.92).clamp(0.0, 1.0)
                }
            }
            _ => 1.0,
        };

        let capsule_pulse_strength = if self.stage == FloatingStage::MiniCapsuleIdle {
            self.capsule_pulse_start.map_or(0.0, |start| {
                let t = (now.saturating_duration_since(start).as_secs_f32() * 1000.0
                    / CAPSULE_PULSE_MS)
                    .clamp(0.0, 1.0);
                if t < 0.25 {
                    t / 0.25
                } else {
                    1.0 - (t - 0.25) / 0.75
                }
            })
        } else {
            0.0
        };

        let body: AnyElement = match self.stage {
            FloatingStage::MiniCapsuleIdle => {
                let act = Rc::clone(&self.on_action);
                render_idle_mini_capsule(
                    capsule_scale,
                    1.0,
                    capsule_pulse_strength,
                    &palette,
                    Rc::new(move || act(ShellAction::CapsuleExpandFull)),
                )
            }
            FloatingStage::FullPanel => {
                let base_content = match self.page {
                    ActivePage::History => self.history.clone().into_any_element(),
                    ActivePage::Settings => self.settings.clone().map_or_else(
                        || self.history.clone().into_any_element(),
                        |s| s.into_any_element(),
                    ),
                };
                let content_with_morph = if let Some((m, _)) = morph_sample {
                    div()
                        .w(px(m.inner_w))
                        .h(px(m.inner_h))
                        .flex_shrink_0()
                        .opacity(morph_content_opacity)
                        .child(base_content)
                        .into_any_element()
                } else {
                    base_content
                };
                if let Some(tr) = self.page_transition {
                    let p = tr.eased_progress(now);
                    let offset_x = if tr.to_settings {
                        lerp_f32(MotionSpec::SETTINGS_ENTER_SLIDE_X, 0.0, p)
                    } else {
                        lerp_f32(MotionSpec::SETTINGS_EXIT_SLIDE_X, 0.0, p)
                    };
                    div()
                        .relative()
                        .left(px(offset_x))
                        .size_full()
                        .opacity(p)
                        .child(content_with_morph)
                        .into_any_element()
                } else {
                    content_with_morph
                }
            }
        };

        let shell_radius = if let Some((_, (_, _, r, _, _))) = morph_sample {
            r
        } else {
            match self.stage {
                FloatingStage::MiniCapsuleIdle => 999.0,
                FloatingStage::FullPanel => LayoutSpec::WINDOW_RADIUS,
            }
        };

        let capsule_glass = if is_capsule_mode {
            Some(capsule_liquid_glass_style(
                palette.is_dark,
                self.is_pointer_over,
                is_blind_dragging(),
                capsule_pulse_strength,
            ))
        } else {
            None
        };

        let shell_shadows = capsule_glass
            .as_ref()
            .map_or_else(|| palette.window_shadows(), |glass| glass.shadows.clone());

        let mut shell_el = div()
            .id("clipboardx-window-shell")
            .relative()
            .font_family(crate::theme::ui_font_family())
            .rounded(px(shell_radius))
            .bg(if let Some(glass) = capsule_glass {
                glass.bg
            } else {
                palette.bg_window.into()
            })
            .shadow(shell_shadows)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &gpui_kit::MouseDownEvent, _, _| {
                    this.drag.on_mouse_down(ev.position);
                }),
            )
            .on_mouse_move(cx.listener(|this, ev, window, _| {
                this.drag.on_mouse_move(ev, window);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, _| {
                    this.finish_blind_drag(window);
                }),
            )
            .child(
                div()
                    .size_full()
                    .rounded(px(shell_radius))
                    .overflow_hidden()
                    .child(body),
            )
            .when(
                !is_capsule_mode && locked_size.is_none() && morph_sample.is_none(),
                |el| el.children(window_resize_handles()),
            );

        #[cfg(not(target_os = "windows"))]
        {
            shell_el = shell_el.on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if this.is_pointer_over != *hovered {
                    this.is_pointer_over = *hovered;
                    if *hovered {
                        this.hover_entered_at = Some(Instant::now());
                        this.hover_left_at = None;
                    } else {
                        this.hover_left_at = Some(Instant::now());
                        this.hover_entered_at = None;
                    }
                    cx.notify();
                }
            }));
        }

        let idle_bounce_size = if self.stage == FloatingStage::MiniCapsuleIdle
            && locked_size.is_none()
            && self.capsule_settle_bounce_start.is_some()
        {
            let body_scale = lerp_f32(1.0, capsule_scale, 0.45);
            Some((
                (CAPSULE_IDLE_WIDTH * body_scale).round(),
                (CAPSULE_IDLE_HEIGHT * body_scale).round(),
            ))
        } else {
            None
        };

        if let Some((lw, lh)) = locked_size {
            shell_el = shell_el.w(px(lw)).h(px(lh)).flex_shrink_0();
        } else if let Some((_, (mw, mh, _, _, _))) = morph_sample {
            shell_el = shell_el.w(px(mw)).h(px(mh)).flex_shrink_0();
        } else if let Some((bw, bh)) = idle_bounce_size {
            shell_el = shell_el.w(px(bw)).h(px(bh)).flex_shrink_0();
        } else {
            shell_el = shell_el.size_full();
        }

        let locked_right = self.locked_align_right;
        let morph_right = morph_sample
            .map(|(m, _)| m.align_right)
            .unwrap_or_else(|| self.capsule_anchor_is_right());

        div()
            .id("clipboardx-shadow-host")
            .size_full()
            .p(px(WINDOW_SHADOW_MARGIN))
            .bg(gpui_kit::transparent_black())
            .on_mouse_move(cx.listener(|this, ev, window, _| {
                this.drag.on_mouse_move(ev, window);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, _| {
                    this.finish_blind_drag(window);
                }),
            )
            .when(locked_size.is_some(), |el| {
                el.flex()
                    .items_center()
                    .when(locked_right, |d| d.justify_end())
                    .when(!locked_right, |d| d.justify_start())
            })
            .when(locked_size.is_none() && morph_sample.is_some(), |el| {
                el.flex()
                    .items_center()
                    .when(morph_right, |d| d.justify_end())
                    .when(!morph_right, |d| d.justify_start())
            })
            .when(
                locked_size.is_none() && morph_sample.is_none() && idle_bounce_size.is_some(),
                |el| el.flex().items_center().justify_center(),
            )
            .child(shell_el)
    }
}

fn window_resize_handles() -> Vec<AnyElement> {
    const EDGE: f32 = 4.0;
    const CORNER: f32 = 10.0;

    vec![
        resize_handle(
            "resize-n",
            WindowResizeEdge::Top,
            ResizeEdge::Top,
            CursorStyle::ResizeUpDown,
            |d| d.top_0().left(px(CORNER)).right(px(CORNER)).h(px(EDGE)),
        ),
        resize_handle(
            "resize-s",
            WindowResizeEdge::Bottom,
            ResizeEdge::Bottom,
            CursorStyle::ResizeUpDown,
            |d| d.bottom_0().left(px(CORNER)).right(px(CORNER)).h(px(EDGE)),
        ),
        resize_handle(
            "resize-w",
            WindowResizeEdge::Left,
            ResizeEdge::Left,
            CursorStyle::ResizeLeftRight,
            |d| d.left_0().top(px(CORNER)).bottom(px(CORNER)).w(px(EDGE)),
        ),
        resize_handle(
            "resize-e",
            WindowResizeEdge::Right,
            ResizeEdge::Right,
            CursorStyle::ResizeLeftRight,
            |d| d.right_0().top(px(CORNER)).bottom(px(CORNER)).w(px(EDGE)),
        ),
        resize_handle(
            "resize-nw",
            WindowResizeEdge::TopLeft,
            ResizeEdge::TopLeft,
            CursorStyle::ResizeUpLeftDownRight,
            |d| d.top_0().left_0().size(px(CORNER)),
        ),
        resize_handle(
            "resize-ne",
            WindowResizeEdge::TopRight,
            ResizeEdge::TopRight,
            CursorStyle::ResizeUpRightDownLeft,
            |d| d.top_0().right_0().size(px(CORNER)),
        ),
        resize_handle(
            "resize-sw",
            WindowResizeEdge::BottomLeft,
            ResizeEdge::BottomLeft,
            CursorStyle::ResizeUpRightDownLeft,
            |d| d.bottom_0().left_0().size(px(CORNER)),
        ),
        resize_handle(
            "resize-se",
            WindowResizeEdge::BottomRight,
            ResizeEdge::BottomRight,
            CursorStyle::ResizeUpLeftDownRight,
            |d| d.bottom_0().right_0().size(px(CORNER)),
        ),
    ]
}

fn resize_handle(
    id: &'static str,
    native_edge: WindowResizeEdge,
    gpui_edge: ResizeEdge,
    cursor: CursorStyle,
    position: impl FnOnce(gpui_kit::Stateful<gpui_kit::Div>) -> gpui_kit::Stateful<gpui_kit::Div>,
) -> AnyElement {
    position(div().id(id).absolute().cursor(cursor))
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            #[cfg(target_os = "windows")]
            {
                let _ = (window, gpui_edge);
                begin_native_window_resize(native_edge);
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = native_edge;
                window.start_window_resize(gpui_edge);
            }
        })
        .into_any_element()
}

/// Coordinates the single resident borderless floating window, custom shadow passthrough,
/// docked oval capsule hover/expand lifecycle, non-activating
/// reusable image hover preview popup, custom GPUI tray context menu, dual-mode hotkeys, and live UI sync.
pub struct ShellController {
    runtime: Rc<AppRuntime>,
    active_window: Option<(AnyWindowHandle, Entity<AppWorkspaceView>)>,
    tray_menu_window: Option<(AnyWindowHandle, Instant)>,
    hover_preview_window: Option<(AnyWindowHandle, Entity<ImageHoverPreviewView>)>,
    hover_preview_visible: bool,
    hover_preview_active_id: Option<ClipItemId>,
    action_rx: mpsc::Receiver<ShellAction>,
    action_tx: mpsc::Sender<ShellAction>,
    tray_menu_rx: mpsc::Receiver<TrayAction>,
    tray_menu_tx: mpsc::Sender<TrayAction>,
    wake_drain: Rc<dyn Fn()>,
    last_window_focused: bool,
    blur_started_at: Option<Instant>,
    suppress_focus_expand_until: Option<Instant>,
    suppress_blur_until: Option<Instant>,
    quiet_idle_since: Option<Instant>,
    idle_reclaimed: bool,
    startup_trim_at: Option<Instant>,
}

impl ShellController {
    /// Launches the background OS event pump and displays the initial popup window.
    pub fn start(runtime: Rc<AppRuntime>, cx: &mut App) {
        let (action_tx, action_rx) = mpsc::channel();
        let (tray_menu_tx, tray_menu_rx) = mpsc::channel();
        let controller = Rc::new(RefCell::new(Self {
            runtime,
            active_window: None,
            tray_menu_window: None,
            hover_preview_window: None,
            hover_preview_visible: false,
            hover_preview_active_id: None,
            action_rx,
            action_tx,
            tray_menu_rx,
            tray_menu_tx,
            wake_drain: Rc::new(|| {}),
            last_window_focused: true,
            blur_started_at: None,
            suppress_focus_expand_until: None,
            suppress_blur_until: Some(Instant::now() + Duration::from_millis(400)),
            quiet_idle_since: None,
            idle_reclaimed: false,
            startup_trim_at: Some(Instant::now() + Duration::from_millis(450)),
        }));

        // Zero-latency microtask dispatcher: wakes immediately on UI hover/tab/click actions
        // without waiting for the background OS polling loop or risking RefCell re-entrancy.
        let async_app = cx.to_async();
        let wake_ctrl = Rc::clone(&controller);
        let wake_pending = Rc::new(Cell::new(false));
        let wake_drain: Rc<dyn Fn()> = Rc::new(move || {
            if wake_pending.replace(true) {
                return;
            }
            let ctrl = Rc::clone(&wake_ctrl);
            let flag = Rc::clone(&wake_pending);
            async_app
                .spawn(async move |cx| {
                    flag.set(false);
                    cx.update(|cx| {
                        ctrl.borrow_mut().drain_pending_actions(cx);
                    });
                })
                .detach();
        });
        controller.borrow_mut().wake_drain = wake_drain;

        controller.borrow_mut().show_panel(cx);

        let pump_ctrl = Rc::clone(&controller);
        cx.spawn(async move |cx| {
            loop {
                let interval = cx.update(|cx| pump_ctrl.borrow().recommended_poll_interval(cx));
                cx.background_executor().timer(interval).await;

                cx.update(|cx| {
                    pump_ctrl.borrow_mut().tick(cx);
                });
            }
        })
        .detach();
    }

    fn is_quiet_standby(&self, cx: &App) -> bool {
        if self.tray_menu_window.is_some() || self.hover_preview_visible || is_blind_dragging() {
            return false;
        }
        let needs_hotkey_poll = self
            .runtime
            .trigger_sm
            .lock()
            .map(|sm| sm.needs_modifier_polling())
            .unwrap_or(false);
        if needs_hotkey_poll {
            return false;
        }
        match &self.active_window {
            None => true,
            Some((_, view)) => view.read(cx).is_quiet_capsule_idle(),
        }
    }

    fn recommended_poll_interval(&self, cx: &App) -> Duration {
        if self.is_quiet_standby(cx) {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(16)
        }
    }

    fn close_tray_menu(&mut self, cx: &mut App) {
        if let Some((handle, _)) = self.tray_menu_window.take() {
            let _ = handle.update(cx, |_, window, _| {
                window.remove_window();
            });
        }
    }

    /// Hides the hover preview popup without destroying its OS window or GPU swapchain
    /// on Windows so hovering the next image item displays instantaneously (< 1ms).
    fn close_hover_preview(&mut self, cx: &mut App) {
        self.hover_preview_active_id = None;
        if !self.hover_preview_visible {
            return;
        }
        self.hover_preview_visible = false;
        #[cfg(target_os = "windows")]
        {
            let _ = cx;
            hide_hover_preview_popup();
        }
        #[cfg(not(target_os = "windows"))]
        {
            if let Some((handle, _)) = self.hover_preview_window.take() {
                let _ = handle.update(cx, |_, window, _| {
                    window.remove_window();
                });
            }
            reset_hover_preview_popup_hwnd();
        }
    }

    /// Permanently tears down the cached hover preview popup window on application quit.
    fn destroy_hover_preview(&mut self, cx: &mut App) {
        self.hover_preview_active_id = None;
        self.hover_preview_visible = false;
        if let Some((handle, _)) = self.hover_preview_window.take() {
            let _ = handle.update(cx, |_, window, _| {
                window.remove_window();
            });
        }
        reset_hover_preview_popup_hwnd();
    }

    fn request_hover_preview(&mut self, item_id: Option<ClipItemId>, cx: &mut App) {
        match item_id {
            Some(id) => {
                if self.hover_preview_visible && self.hover_preview_active_id == Some(id) {
                    return;
                }
                self.show_hover_preview_now(id, cx);
            }
            None => {
                self.close_hover_preview(cx);
            }
        }
    }

    fn show_hover_preview_now(&mut self, id: ClipItemId, cx: &mut App) {
        if self.tray_menu_window.is_some() {
            return;
        }
        let is_morphing = self
            .active_window
            .as_ref()
            .is_some_and(|(_, view)| view.read(cx).is_morphing_or_locked());
        if is_morphing {
            self.close_hover_preview(cx);
            return;
        }
        let Some(cached) = self.runtime.get_or_load_image_thumb(id) else {
            self.close_hover_preview(cx);
            return;
        };
        let geom = compute_hover_card_geometry(cached.orig_w, cached.orig_h);
        let row_center_y = cursor_screen_y().unwrap_or(300);

        let scale = self
            .active_window
            .as_ref()
            .and_then(|(h, _)| {
                let mut s = 1.0_f32;
                h.update(cx, |_, win, _| s = win.scale_factor()).ok()?;
                Some(s)
            })
            .unwrap_or(1.0);

        let phys_canvas_w = (HOVER_IMG_MAX_WIDTH * scale).round() as i32;
        let phys_canvas_h = (HOVER_IMG_MAX_HEIGHT * scale).round() as i32;
        let phys_card_w = (geom.card_w * scale).round() as i32;
        let phys_margin = (WINDOW_SHADOW_MARGIN * scale).round() as i32;
        let phys_gap = (HOVER_CARD_GAP * scale).round() as i32;

        if let Some((handle, view)) = &self.hover_preview_window {
            let view = view.clone();
            let animate_entrance = !self.hover_preview_visible;
            let on_left = position_hover_preview_popup(
                phys_canvas_w,
                phys_canvas_h,
                phys_card_w,
                phys_margin,
                row_center_y,
                phys_gap,
            );
            let spec = HoverImageSpec {
                image: Arc::clone(&cached.image),
                geometry: geom,
                opened_on_left: on_left,
            };
            let ok = handle
                .update(cx, |_, _, cx| {
                    view.update(cx, |v, cx| v.update_spec(spec, animate_entrance, cx));
                })
                .is_ok();
            if ok {
                self.hover_preview_visible = true;
                self.hover_preview_active_id = Some(id);
                return;
            }
            self.hover_preview_window = None;
            self.hover_preview_visible = false;
            reset_hover_preview_popup_hwnd();
        }

        let pad = WINDOW_SHADOW_MARGIN * 2.0;
        let total_w = HOVER_IMG_MAX_WIDTH + pad;
        let total_h = HOVER_IMG_MAX_HEIGHT + pad;
        let (bounds, initial_on_left) =
            if let Some((main_l, main_t, main_r, main_b)) = own_window_visual_rect() {
                let inv_scale = 1.0 / scale.max(0.5);
                let fits_left = main_l as f32 * inv_scale - HOVER_CARD_GAP - geom.card_w >= 4.0;
                let (vis_x, on_left) = if fits_left {
                    (
                        main_l as f32 * inv_scale - HOVER_CARD_GAP - HOVER_IMG_MAX_WIDTH,
                        true,
                    )
                } else {
                    (main_r as f32 * inv_scale + HOVER_CARD_GAP, false)
                };
                let anchor_y = row_center_y.clamp(main_t, main_b) as f32 * inv_scale;
                let vis_y = (anchor_y - HOVER_IMG_MAX_HEIGHT * 0.5).max(4.0);
                (
                    WindowBounds::Windowed(Bounds::new(
                        point(
                            px(vis_x - WINDOW_SHADOW_MARGIN),
                            px(vis_y - WINDOW_SHADOW_MARGIN),
                        ),
                        size(px(total_w), px(total_h)),
                    )),
                    on_left,
                )
            } else {
                (
                    WindowBounds::centered(size(px(total_w), px(total_h)), cx),
                    true,
                )
            };

        let options = WindowOptions {
            window_bounds: Some(bounds),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            // Disable GPUI's default 30fps inactive-window throttle so unfocused hover image
            // popups animate at the display's native refresh rate.
            inactive_frame_interval: None,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };

        let runtime = Rc::clone(&self.runtime);
        let mut built_view: Option<Entity<ImageHoverPreviewView>> = None;
        let initial_spec = HoverImageSpec {
            image: cached.image,
            geometry: geom,
            opened_on_left: initial_on_left,
        };
        let opened = cx.open_window(options, |window, cx| {
            let view = cx.new(|cx| ImageHoverPreviewView::new(runtime, initial_spec, cx));
            built_view = Some(view.clone());
            cx.new(|cx| {
                gpui_kit::base::Root::new(view, window, cx).bg(gpui_kit::transparent_black())
            })
        });

        if let (Ok(handle), Some(view)) = (opened, built_view) {
            let on_left = position_hover_preview_popup(
                phys_canvas_w,
                phys_canvas_h,
                phys_card_w,
                phys_margin,
                row_center_y,
                phys_gap,
            );
            if on_left != initial_on_left {
                let _ = handle.update(cx, |_, _, cx| {
                    view.update(cx, |v, cx| v.set_opened_on_left(on_left, cx));
                });
            }
            self.hover_preview_window = Some((handle.into(), view));
            self.hover_preview_visible = true;
            self.hover_preview_active_id = Some(id);
        }
    }

    fn open_tray_menu(&mut self, anchor_x: i32, anchor_y: i32, cx: &mut App) {
        self.close_hover_preview(cx);
        self.close_tray_menu(cx);

        let pad = WINDOW_SHADOW_MARGIN * 2.0;
        let bounds = WindowBounds::centered(
            size(px(TRAY_MENU_WIDTH + pad), px(TRAY_MENU_HEIGHT + pad)),
            cx,
        );
        let options = WindowOptions {
            window_bounds: Some(bounds),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };

        let runtime = Rc::clone(&self.runtime);
        let tx = self.tray_menu_tx.clone();
        let wake_tray = Rc::clone(&self.wake_drain);
        let opened = cx.open_window(options, |window, cx| {
            let scale = window.scale_factor();
            let phys_w = (TRAY_MENU_WIDTH * scale).round() as i32;
            let phys_h = (TRAY_MENU_HEIGHT * scale).round() as i32;
            let phys_margin = (WINDOW_SHADOW_MARGIN * scale).round() as i32;
            cx.spawn(async move |_| {
                configure_transient_popup_window(anchor_x, anchor_y, phys_w, phys_h, phys_margin);
            })
            .detach();

            let view = cx.new(|cx| {
                TrayMenuView::new(
                    runtime,
                    Rc::new(move |act| {
                        let _ = tx.send(act);
                        wake_tray();
                    }),
                    window,
                    cx,
                )
            });
            cx.new(|cx| {
                gpui_kit::base::Root::new(view, window, cx).bg(gpui_kit::transparent_black())
            })
        });

        match opened {
            Ok(handle) => {
                self.tray_menu_window = Some((handle.into(), Instant::now()));
            }
            Err(err) => {
                tracing::warn!(error = %err, "failed to open custom tray context menu");
            }
        }
    }

    fn ensure_window(&mut self, open_settings_immediately: bool, cx: &mut App) {
        self.quiet_idle_since = None;
        self.idle_reclaimed = false;
        self.close_hover_preview(cx);
        self.close_tray_menu(cx);
        self.suppress_blur_until = Some(Instant::now() + Duration::from_millis(380));
        self.blur_started_at = None;

        if let Some((handle, view)) = &self.active_window {
            let view = view.clone();
            let res = handle.update(cx, |_, window, cx| {
                view.update(cx, |ws, cx| {
                    ws.restore_full_panel(true, false, window, cx);
                    if open_settings_immediately {
                        ws.open_settings(window, cx);
                    } else if ws.history_snapshot_dirty {
                        ws.history_snapshot_dirty = false;
                        ws.history.update(cx, |h, cx| h.refresh_items(cx));
                    }
                });
                window.activate_window();
                remember_own_window_hwnd();
                sync_native_shadow_region(window, LayoutSpec::WINDOW_RADIUS);
            });
            if res.is_ok() {
                self.last_window_focused = true;
                return;
            }
            self.active_window = None;
        }

        self.runtime.remember_foreground_window();

        let cfg = self.runtime.snapshot_config();
        let visual_w = if open_settings_immediately || !cfg.is_compact_mode {
            LayoutSpec::WINDOW_DUAL_WIDTH
        } else {
            LayoutSpec::WINDOW_SLIM_WIDTH
        };
        let pad = WINDOW_SHADOW_MARGIN * 2.0;

        let bounds = WindowBounds::centered(
            size(px(visual_w + pad), px(LayoutSpec::WINDOW_HEIGHT + pad)),
            cx,
        );
        let options = WindowOptions {
            window_bounds: Some(bounds),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: true,
            is_resizable: true,
            is_minimizable: false,
            // Disable GPUI's default 30fps inactive-window throttle so hover-expanding from the
            // unfocused mini-capsule animates at the exact same native refresh rate as `Alt+V`.
            inactive_frame_interval: None,
            window_min_size: Some(size(
                px(CAPSULE_IDLE_WIDTH + pad),
                px(CAPSULE_IDLE_HEIGHT + pad),
            )),
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };

        let runtime = Rc::clone(&self.runtime);
        let tx = self.action_tx.clone();
        let wake_actions = Rc::clone(&self.wake_drain);
        let mut built_view: Option<Entity<AppWorkspaceView>> = None;
        let opened = cx.open_window(options, |window, cx| {
            remember_own_window_hwnd();
            sync_native_shadow_region(window, LayoutSpec::WINDOW_RADIUS);
            let rt = Rc::clone(&runtime);
            let act_tx = tx.clone();
            let wake = Rc::clone(&wake_actions);
            let view = cx.new(|cx| {
                let mut ws = AppWorkspaceView::new(
                    rt,
                    Rc::new(move |act| {
                        let _ = act_tx.send(act);
                        wake();
                    }),
                    window,
                    cx,
                );
                if open_settings_immediately {
                    ws.open_settings(window, cx);
                }
                ws
            });
            built_view = Some(view.clone());
            cx.new(|cx| {
                gpui_kit::base::Root::new(view, window, cx).bg(gpui_kit::transparent_black())
            })
        });

        match (opened, built_view) {
            (Ok(handle), Some(view)) => {
                self.active_window = Some((handle.into(), view));
                self.last_window_focused = true;
                if cfg.blur_behavior != WindowBlurBehavior::AutoHide {
                    set_own_window_topmost(true);
                }
            }
            (Err(err), _) => {
                tracing::error!(error = %err, "failed to open ClipboardX floating panel");
            }
            (Ok(_), None) => {
                tracing::error!("floating panel opened without constructing workspace view");
            }
        }
    }

    fn show_panel(&mut self, cx: &mut App) {
        self.ensure_window(false, cx);
    }

    fn show_settings_inline(&mut self, cx: &mut App) {
        self.ensure_window(true, cx);
    }

    fn hide_panel(&mut self, cx: &mut App) {
        self.destroy_hover_preview(cx);
        if let Some((handle, _)) = self.active_window.take() {
            let _ = handle.update(cx, |_, window, _cx| {
                window.remove_window();
            });
        }
        self.last_window_focused = false;
        self.blur_started_at = None;
        if let Ok(mut sm) = self.runtime.trigger_sm.lock() {
            sm.mark_hidden();
        }
        self.quiet_idle_since = None;
        self.idle_reclaimed = false;
    }

    fn enter_standby_for_config(&mut self, cx: &mut App) {
        self.destroy_hover_preview(cx);
        if let Ok(mut sm) = self.runtime.trigger_sm.lock() {
            sm.mark_hidden();
        }
        let cfg = self.runtime.snapshot_config();
        match cfg.blur_behavior {
            WindowBlurBehavior::DockRightCapsule => {
                if let Some((handle, view)) = &self.active_window {
                    let view = view.clone();
                    let _ = handle.update(cx, |_, window, cx| {
                        view.update(cx, |ws, cx| ws.enter_mini_capsule_idle(window, cx));
                    });
                }
                self.quiet_idle_since = None;
                self.idle_reclaimed = false;
            }
            WindowBlurBehavior::AutoHide => {
                self.hide_panel(cx);
            }
        }
    }

    /// Reclaims transient preview caches, decoded thumbnail textures, SQLite page buffers,
    /// and idle OS working-set pages once the window settles into capsule or tray standby.
    fn perform_idle_memory_reclaim(&mut self, cx: &mut App) {
        let can_reclaim = match &self.active_window {
            None => true,
            Some((_, view)) => view.read(cx).is_quiet_capsule_idle(),
        };
        if !can_reclaim {
            return;
        }
        self.destroy_hover_preview(cx);

        let mut images = self.runtime.drain_image_cache();
        if let Some((handle, view)) = &self.active_window {
            let view = view.clone();
            let _ = handle.update(cx, |_, _, cx| {
                if let Some(extra) = view.update(cx, |ws, cx| ws.reclaim_idle_caches(cx)) {
                    images.push(extra);
                }
            });
        }
        drop(images);

        self.runtime.reclaim_storage_memory();
        trim_process_working_set();
    }

    fn handle_after_paste(&mut self, cx: &mut App) {
        self.close_hover_preview(cx);
        let is_pinned = self
            .active_window
            .as_ref()
            .is_some_and(|(_, view)| view.read(cx).is_window_pinned());
        if is_pinned {
            return;
        }
        let cfg = self.runtime.snapshot_config();
        if cfg.blur_behavior == WindowBlurBehavior::AutoHide {
            self.hide_panel(cx);
            return;
        }
        self.suppress_focus_expand_until = Some(Instant::now() + Duration::from_millis(420));
        self.last_window_focused = false;
        self.blur_started_at = None;
        self.enter_standby_for_config(cx);
    }

    fn handle_tray_action(&mut self, action: TrayAction, cx: &mut App) {
        match action {
            TrayAction::ShowOrHide => {
                self.close_tray_menu(cx);
                let is_full_visible_and_focused = self.last_window_focused
                    && self
                        .active_window
                        .as_ref()
                        .is_some_and(|(_, view)| view.read(cx).stage == FloatingStage::FullPanel);
                if is_full_visible_and_focused {
                    let blur_mode = self.runtime.snapshot_config().blur_behavior;
                    if blur_mode == WindowBlurBehavior::AutoHide {
                        self.hide_panel(cx);
                    } else {
                        self.last_window_focused = false;
                        self.blur_started_at = None;
                        self.suppress_focus_expand_until =
                            Some(Instant::now() + Duration::from_millis(360));
                        self.enter_standby_for_config(cx);
                    }
                } else {
                    self.show_panel(cx);
                }
            }
            TrayAction::OpenMenu { x, y } => {
                self.open_tray_menu(x, y, cx);
            }
            TrayAction::TogglePause => {
                self.close_tray_menu(cx);
                let _ = self.runtime.toggle_pause();
                self.runtime.notify_ui(UiEvent::HistoryUpdated);
            }
            TrayAction::OpenSettings => {
                self.show_settings_inline(cx);
            }
            TrayAction::Quit => {
                self.destroy_hover_preview(cx);
                self.close_tray_menu(cx);
                cx.quit();
            }
        }
    }

    fn poll_tray_menu_focus(&mut self, cx: &mut App) {
        let Some((handle, opened_at)) = self.tray_menu_window else {
            return;
        };
        if opened_at.elapsed() < Duration::from_millis(220) {
            return;
        }
        let mut still_active = false;
        let ok = handle
            .update(cx, |_, window, _| {
                still_active = window.is_window_active();
            })
            .is_ok();
        if !ok {
            self.tray_menu_window = None;
        } else if !still_active {
            self.close_tray_menu(cx);
        }
    }

    fn poll_focus_and_docking(&mut self, cx: &mut App) {
        if self.tray_menu_window.is_some() {
            return;
        }
        if is_blind_dragging() {
            if !is_left_mouse_button_down()
                && let Some((handle, view)) = self.active_window.as_ref().cloned()
            {
                let _ = handle.update(cx, |_, window, cx| {
                    view.update(cx, |ws, cx| {
                        ws.finish_blind_drag(window);
                        cx.notify();
                    });
                });
            } else {
                return;
            }
        }

        #[cfg(target_os = "windows")]
        let os_fg = is_own_process_foreground();
        #[cfg(not(target_os = "windows"))]
        let os_fg = false;

        if !os_fg {
            self.runtime.remember_foreground_window();
        }

        let Some((handle, view)) = self.active_window.as_ref().cloned() else {
            return;
        };

        let now = Instant::now();
        let cfg = self.runtime.snapshot_config();

        let (
            ws_stage,
            ws_pointer_over,
            ws_hover_entered,
            ws_hover_left,
            ws_hover_expanded,
            on_settings,
            window_pinned,
        ) = {
            let ws = view.read(cx);
            (
                ws.stage,
                ws.is_pointer_over,
                ws.hover_entered_at,
                ws.hover_left_at,
                ws.hover_expanded_panel,
                ws.is_on_settings(),
                ws.is_window_pinned(),
            )
        };

        let in_capsule = ws_stage == FloatingStage::MiniCapsuleIdle;
        let cursor_over_opt = is_cursor_over_own_window();
        let hover_state_changed = cursor_over_opt.is_some_and(|over| over != ws_pointer_over);
        let capsule_timer_pending = (in_capsule && ws_pointer_over && ws_hover_entered.is_some())
            || (ws_stage == FloatingStage::FullPanel
                && ws_hover_expanded
                && !window_pinned
                && !ws_pointer_over
                && ws_hover_left.is_some());

        let mut left_window = false;
        #[cfg(target_os = "windows")]
        let now_focused = {
            if os_fg {
                remember_own_window_hwnd();
            }
            if hover_state_changed || capsule_timer_pending {
                let ok = handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |ws, cx| {
                            left_window = ws.tick_capsule_hover(now, window, cx);
                        });
                    })
                    .is_ok();
                if !ok {
                    self.active_window = None;
                    self.close_hover_preview(cx);
                    return;
                }
            }
            os_fg
        };

        #[cfg(not(target_os = "windows"))]
        let now_focused = {
            let mut active = false;
            let ok = handle
                .update(cx, |_, window, cx| {
                    active = window.is_window_active();
                    if hover_state_changed || capsule_timer_pending {
                        view.update(cx, |ws, cx| {
                            left_window = ws.tick_capsule_hover(now, window, cx);
                        });
                    }
                })
                .is_ok();
            if !ok {
                self.active_window = None;
                self.close_hover_preview(cx);
                return;
            }
            active
        };

        if left_window {
            self.close_hover_preview(cx);
        }

        if now_focused {
            self.blur_started_at = None;
            if !self.last_window_focused {
                self.last_window_focused = true;
                let allow_expand = !in_capsule
                    && self
                        .suppress_focus_expand_until
                        .is_none_or(|until| now >= until);
                let _ = handle.update(cx, |_, window, cx| {
                    view.update(cx, |ws, cx| {
                        ws.restore_full_panel(allow_expand, false, window, cx);
                    });
                });
            }
        } else {
            if window_pinned {
                self.blur_started_at = None;
                self.last_window_focused = false;
                return;
            }
            if self.suppress_blur_until.is_some_and(|until| now < until) {
                return;
            }
            let blur_since = *self.blur_started_at.get_or_insert(now);
            if now.duration_since(blur_since) < Duration::from_millis(120) {
                return;
            }
            if self.last_window_focused {
                self.last_window_focused = false;
                if cfg.blur_behavior == WindowBlurBehavior::AutoHide {
                    if !on_settings {
                        self.hide_panel(cx);
                    }
                } else if !in_capsule {
                    self.enter_standby_for_config(cx);
                }
            }
        }
    }

    fn drain_pending_actions(&mut self, cx: &mut App) {
        while let Ok(act) = self.tray_menu_rx.try_recv() {
            self.handle_tray_action(act, cx);
        }

        while let Ok(act) = self.action_rx.try_recv() {
            match act {
                ShellAction::Panel(PanelRequest::HideWindow) => {
                    self.close_hover_preview(cx);
                    if let Some((handle, view)) = &self.active_window {
                        let view = view.clone();
                        let _ = handle.update(cx, |_, _, cx| {
                            view.update(cx, |ws, cx| {
                                if ws.is_window_pinned() {
                                    ws.toggle_window_pin(cx);
                                }
                            });
                        });
                    }
                    let blur_mode = self.runtime.snapshot_config().blur_behavior;
                    if blur_mode == WindowBlurBehavior::DockRightCapsule {
                        let prev_token = self
                            .runtime
                            .last_foreground
                            .lock()
                            .ok()
                            .and_then(|guard| *guard);
                        if let Some(target) = prev_token {
                            let _ = self.runtime.platform.restore_foreground_window(target);
                        }
                        self.last_window_focused = false;
                        self.blur_started_at = None;
                        self.suppress_focus_expand_until =
                            Some(Instant::now() + Duration::from_millis(520));
                        self.enter_standby_for_config(cx);
                    } else {
                        self.hide_panel(cx);
                    }
                }
                ShellAction::Panel(PanelRequest::AfterPaste) => self.handle_after_paste(cx),
                ShellAction::Panel(PanelRequest::OpenSettings) => self.show_settings_inline(cx),
                ShellAction::Panel(PanelRequest::ToggleCompact) => {
                    self.close_hover_preview(cx);
                    if let Some((handle, view)) = &self.active_window {
                        let view = view.clone();
                        let _ = handle.update(cx, |_, window, cx| {
                            view.update(cx, |ws, cx| ws.toggle_compact_animated(window, cx));
                        });
                    }
                }
                ShellAction::Panel(PanelRequest::ToggleWindowPin) => {
                    if let Some((handle, view)) = &self.active_window {
                        let view = view.clone();
                        let mut pinned = false;
                        let _ = handle.update(cx, |_, _, cx| {
                            view.update(cx, |ws, cx| {
                                pinned = ws.toggle_window_pin(cx);
                            });
                        });
                        let cfg = self.runtime.snapshot_config();
                        set_own_window_topmost(
                            pinned || cfg.blur_behavior != WindowBlurBehavior::AutoHide,
                        );
                    }
                }
                ShellAction::Panel(PanelRequest::HoverImagePreview(item_id)) => {
                    self.request_hover_preview(item_id, cx);
                }
                ShellAction::CapsuleExpandFull => {
                    self.close_hover_preview(cx);
                    self.show_panel(cx);
                }
            }
        }
    }

    fn tick(&mut self, cx: &mut App) {
        self.poll_tray_menu_focus(cx);
        self.drain_pending_actions(cx);

        self.poll_focus_and_docking(cx);

        let cfg = self.runtime.snapshot_config();
        let (hk_event, toggle_outcome) = self.runtime.handle_hotkey_tick();

        if let Some(ev) = hk_event {
            match ev {
                HotkeyEventKind::TogglePanelPressed => match toggle_outcome {
                    TriggerOutcome::Show => self.show_panel(cx),
                    TriggerOutcome::Hide => {
                        if cfg.blur_behavior == WindowBlurBehavior::AutoHide {
                            self.hide_panel(cx);
                        } else {
                            self.last_window_focused = false;
                            self.blur_started_at = None;
                            self.suppress_focus_expand_until =
                                Some(Instant::now() + Duration::from_millis(360));
                            self.enter_standby_for_config(cx);
                        }
                    }
                    TriggerOutcome::None => {}
                },
                HotkeyEventKind::ToggleCompactPressed => {
                    self.close_hover_preview(cx);
                    if let Some((handle, view)) = &self.active_window {
                        let view = view.clone();
                        let _ = handle.update(cx, |_, window, cx| {
                            view.update(cx, |ws, cx| ws.toggle_compact_animated(window, cx));
                        });
                    }
                }
                HotkeyEventKind::TogglePausePressed => {
                    let _ = self.runtime.toggle_pause();
                    self.runtime.notify_ui(UiEvent::HistoryUpdated);
                }
            }
        }

        let needs_poll = self
            .runtime
            .trigger_sm
            .lock()
            .map(|sm| sm.needs_modifier_polling())
            .unwrap_or(false);
        if needs_poll {
            let held = self
                .runtime
                .platform
                .is_hotkey_modifier_held(&cfg.toggle_hotkey);
            let outcome = self
                .runtime
                .trigger_sm
                .lock()
                .map(|mut sm| {
                    sm.on_modifier_tick(
                        cfg.hotkey_trigger_mode,
                        held,
                        cfg.hold_threshold_ms,
                        Instant::now(),
                    )
                })
                .unwrap_or(TriggerOutcome::None);
            if outcome == TriggerOutcome::Hide {
                if cfg.blur_behavior == WindowBlurBehavior::AutoHide {
                    self.hide_panel(cx);
                } else {
                    self.last_window_focused = false;
                    self.blur_started_at = None;
                    self.suppress_focus_expand_until =
                        Some(Instant::now() + Duration::from_millis(360));
                    self.enter_standby_for_config(cx);
                }
            }
        }

        if let Some(action) = self.runtime.poll_tray_action() {
            self.handle_tray_action(action, cx);
        }

        let updates = self.runtime.drain_ui_events();
        if !updates.is_empty() {
            let config_changed = updates.contains(&UiEvent::ConfigUpdated);
            let history_changed = updates.contains(&UiEvent::HistoryUpdated);
            if config_changed {
                let is_pinned = self
                    .active_window
                    .as_ref()
                    .is_some_and(|(_, view)| view.read(cx).is_window_pinned());
                let topmost = is_pinned || cfg.blur_behavior != WindowBlurBehavior::AutoHide;
                set_own_window_topmost(topmost);
            }
            if let Some((handle, view)) = &self.active_window {
                let view = view.clone();
                let res = handle.update(cx, |_, window, cx| {
                    view.update(cx, |ws, cx| {
                        ws.refresh_events(history_changed, config_changed, window, cx)
                    });
                });
                if res.is_err() {
                    self.active_window = None;
                }
            }
            if history_changed {
                self.quiet_idle_since = None;
                self.idle_reclaimed = false;
            }
        }

        let now = Instant::now();
        if let Some(trim_due) = self.startup_trim_at
            && now >= trim_due
        {
            self.startup_trim_at = None;
            self.runtime.reclaim_storage_memory();
            trim_process_working_set();
        }

        // Unified state-machine idle memory reclamation: catches EVERY return to quiet capsule
        // or tray standby (blur, hover-collapse, Esc, after-paste, or background copy pulse).
        if self.is_quiet_standby(cx) {
            if !self.idle_reclaimed {
                let since = *self.quiet_idle_since.get_or_insert(now);
                if now.duration_since(since) >= Duration::from_millis(600) {
                    self.idle_reclaimed = true;
                    self.perform_idle_memory_reclaim(cx);
                }
            }
        } else {
            self.quiet_idle_since = None;
            self.idle_reclaimed = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_morph_interpolates_dimensions_and_radius_smoothly() {
        let start = Instant::now();
        let morph = ShellMorphAnim {
            from_w: CAPSULE_IDLE_WIDTH,
            from_h: CAPSULE_IDLE_HEIGHT,
            from_r: CAPSULE_IDLE_RADIUS,
            to_w: LayoutSpec::WINDOW_SLIM_WIDTH,
            to_h: LayoutSpec::WINDOW_HEIGHT,
            to_r: LayoutSpec::WINDOW_RADIUS,
            inner_w: LayoutSpec::WINDOW_SLIM_WIDTH,
            inner_h: LayoutSpec::WINDOW_HEIGHT,
            align_right: true,
            fade_content: true,
            completion: MorphCompletion::None,
            started_at: start,
            duration_ms: MotionSpec::FULL_EXPAND_MS,
        };

        let (w0, h0, r0, p0, done0) = morph.sample(start);
        assert_eq!((w0, h0, r0, p0, done0), (38.0, 58.0, 19.0, 0.0, false));

        let (wm, hm, rm, pm, donem) = morph.sample(start + Duration::from_millis(110));
        assert!(wm > CAPSULE_IDLE_WIDTH && wm < LayoutSpec::WINDOW_SLIM_WIDTH);
        assert!(hm > CAPSULE_IDLE_HEIGHT && hm < LayoutSpec::WINDOW_HEIGHT);
        assert!(rm < CAPSULE_IDLE_RADIUS && rm > LayoutSpec::WINDOW_RADIUS);
        assert!(pm > 0.85 && pm < 1.0);
        assert!(!donem);

        let (w1, h1, r1, p1, done1) = morph.sample(start + Duration::from_millis(240));
        assert_eq!(
            (w1, h1, r1, p1, done1),
            (
                LayoutSpec::WINDOW_SLIM_WIDTH,
                LayoutSpec::WINDOW_HEIGHT,
                LayoutSpec::WINDOW_RADIUS,
                1.0,
                true
            )
        );
    }
}
