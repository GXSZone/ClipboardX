use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::{Context, Image, ImageSource, ObjectFit, Render, Window, div, img, px};

use crate::shell::WINDOW_SHADOW_MARGIN;
use crate::state::AppRuntime;
use crate::theme::{MotionSpec, ThemePalette, ease_out_expo_cubic, lerp_f32, resolve_theme_id};

/// Geometry metrics for the borderless image hover preview card
/// (`r = 10`, `gap = 8`, `80px` baseline height, aspect-locked without letterbox bars).
pub const HOVER_IMG_MAX_HEIGHT: f32 = 80.0;
pub const HOVER_IMG_MIN_SIDE: f32 = 24.0;
pub const HOVER_IMG_MAX_WIDTH: f32 = 160.0;
pub const HOVER_CARD_RADIUS: f32 = 10.0;
pub const HOVER_CARD_GAP: f32 = 8.0;

/// Exact aspect-preserving immersive card dimensions for a hovered image item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoverCardGeometry {
    pub card_w: f32,
    pub card_h: f32,
}

/// Computes exact aspect-preserving dimensions for immersive image-only hover preview:
/// targets height `80px` with width up to `160px`; ultrawide images shrink height proportionally
/// instead of adding letterbox bars so the rounded card shows 100% full-bleed image only.
pub fn compute_hover_card_geometry(orig_w: u32, orig_h: u32) -> HoverCardGeometry {
    let w = orig_w.max(1) as f32;
    let h = orig_h.max(1) as f32;
    let raw_w = HOVER_IMG_MAX_HEIGHT * (w / h);

    if raw_w > HOVER_IMG_MAX_WIDTH {
        let fitted_h = (HOVER_IMG_MAX_WIDTH * (h / w))
            .round()
            .clamp(HOVER_IMG_MIN_SIDE, HOVER_IMG_MAX_HEIGHT);
        HoverCardGeometry {
            card_w: HOVER_IMG_MAX_WIDTH,
            card_h: fitted_h,
        }
    } else {
        let fitted_w = raw_w.round().clamp(HOVER_IMG_MIN_SIDE, HOVER_IMG_MAX_WIDTH);
        HoverCardGeometry {
            card_w: fitted_w,
            card_h: HOVER_IMG_MAX_HEIGHT,
        }
    }
}

/// Active payload displayed inside the immersive image hover preview window.
#[derive(Clone)]
pub struct HoverImageSpec {
    pub image: Arc<Image>,
    pub geometry: HoverCardGeometry,
    pub opened_on_left: bool,
}

/// Borderless, non-activating, click-through GPUI view showing only the full-bleed rounded image
/// with exact aspect ratio and a VSync-driven 220ms slide and fade entrance.
pub struct ImageHoverPreviewView {
    runtime: Rc<AppRuntime>,
    spec: HoverImageSpec,
    entrance_started_at: Option<Instant>,
}

impl ImageHoverPreviewView {
    pub fn new(runtime: Rc<AppRuntime>, spec: HoverImageSpec, cx: &mut Context<'_, Self>) -> Self {
        cx.notify();
        Self {
            runtime,
            spec,
            entrance_started_at: Some(Instant::now()),
        }
    }

    /// Updates the displayed image spec. Replays the 220ms entrance only when appearing from hidden;
    /// scrubbing between adjacent image rows swaps immediately without fading to transparent.
    pub fn update_spec(
        &mut self,
        spec: HoverImageSpec,
        animate_entrance: bool,
        cx: &mut Context<'_, Self>,
    ) {
        self.spec = spec;
        if animate_entrance {
            self.entrance_started_at = Some(Instant::now());
        } else {
            self.entrance_started_at = None;
        }
        cx.notify();
    }

    pub fn set_opened_on_left(&mut self, on_left: bool, cx: &mut Context<'_, Self>) {
        if self.spec.opened_on_left != on_left {
            self.spec.opened_on_left = on_left;
            cx.notify();
        }
    }
}

impl Render for ImageHoverPreviewView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        let cfg = self.runtime.snapshot_config();
        let theme_id = resolve_theme_id(&cfg, window.appearance());
        let palette = ThemePalette::from_theme_id(theme_id);
        let geom = self.spec.geometry;

        let p = if let Some(started_at) = self.entrance_started_at {
            let raw = (started_at.elapsed().as_secs_f32() * 1000.0
                / MotionSpec::IMAGE_HOVER_ENTER_MS)
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

        // 220ms ease-out slide with edge-locked alignment inside the fixed 160x80 visual canvas
        // so switching aspect ratios never shifts or overlaps the main panel.
        let on_left = self.spec.opened_on_left;
        let slide_from = if on_left {
            MotionSpec::IMAGE_HOVER_SLIDE_X
        } else {
            -MotionSpec::IMAGE_HOVER_SLIDE_X
        };
        let slide_x = lerp_f32(slide_from, 0.0, p);
        let cur_opacity = lerp_f32(0.55, 1.0, p);

        div()
            .id("image-hover-preview-host")
            .size_full()
            .p(px(WINDOW_SHADOW_MARGIN))
            .bg(gpui_kit::transparent_black())
            .flex()
            .items_center()
            .when(on_left, |d| d.justify_end())
            .when(!on_left, |d| d.justify_start())
            .child(
                div()
                    .id("image-hover-preview-card")
                    .relative()
                    .left(px(slide_x))
                    .w(px(geom.card_w))
                    .h(px(geom.card_h))
                    .opacity(cur_opacity)
                    .rounded(px(HOVER_CARD_RADIUS))
                    .shadow(palette.window_shadows())
                    .overflow_hidden()
                    .child(
                        img(ImageSource::Image(Arc::clone(&self.spec.image)))
                            .size_full()
                            .rounded(px(HOVER_CARD_RADIUS))
                            .object_fit(ObjectFit::Cover),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_immersive_aspect_preserving_card_dimensions() {
        // 16:9 -> 142x80 full-bleed
        let g16_9 = compute_hover_card_geometry(1920, 1080);
        assert_eq!(g16_9.card_w, 142.0);
        assert_eq!(g16_9.card_h, 80.0);

        // 1362x735 -> 148x80 full-bleed
        let g22 = compute_hover_card_geometry(1362, 735);
        assert_eq!(g22.card_w, 148.0);
        assert_eq!(g22.card_h, 80.0);

        // 1:1 -> 80x80
        let g1_1 = compute_hover_card_geometry(1024, 1024);
        assert_eq!(g1_1.card_w, 80.0);
        assert_eq!(g1_1.card_h, 80.0);

        // 9:16 -> 45x80
        let g9_16 = compute_hover_card_geometry(1080, 1920);
        assert_eq!(g9_16.card_w, 45.0);
        assert_eq!(g9_16.card_h, 80.0);

        // 32:9 ultrawide -> 160x45 (no letterbox bars, exact 32:9 aspect ratio)
        let g32_9 = compute_hover_card_geometry(3840, 1080);
        assert_eq!(g32_9.card_w, 160.0);
        assert_eq!(g32_9.card_h, 45.0);
    }
}
