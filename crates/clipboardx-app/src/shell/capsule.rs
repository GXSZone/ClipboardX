use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Background, BoxShadow, Hsla, Rgba, div, linear_color_stop, linear_gradient, point,
    px,
};

use crate::icons::{
    brand_capsule_mark, is_blind_dragging, reset_blind_drag_click_guard, was_blind_drag_consumed,
};
use crate::theme::ThemePalette;

/// Logical dimensions for the vertical pill mini-capsule (`38x58`, `r = 999`).
pub const CAPSULE_IDLE_WIDTH: f32 = 38.0;
pub const CAPSULE_IDLE_HEIGHT: f32 = 58.0;

/// Duration of the capture-confirmation pulse feedback after a new clip arrives.
pub const CAPSULE_PULSE_MS: f32 = 650.0;

/// Borderless liquid optical glass material parameters for the mini-capsule shell.
pub struct CapsuleLiquidGlassStyle {
    pub bg: Background,
    pub shadows: Vec<BoxShadow>,
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

fn glass_gradient(top_left: Hsla, bottom_right: Hsla) -> Background {
    linear_gradient(
        135.0,
        linear_color_stop(top_left, 0.0),
        linear_color_stop(bottom_right, 1.0),
    )
}

/// Computes borderless liquid glass linear gradient, inset specular refraction shadows,
/// and dual-layer ambient drop shadows for the current theme and interaction state.
pub fn capsule_liquid_glass_style(
    is_dark: bool,
    hovered: bool,
    dragging: bool,
    pulse_strength: f32,
) -> CapsuleLiquidGlassStyle {
    let pulse = pulse_strength.clamp(0.0, 1.0);
    if is_dark {
        if dragging {
            CapsuleLiquidGlassStyle {
                bg: glass_gradient(rgba_hex(0x313B52, 0.70), rgba_hex(0x171D2A, 0.58)),
                shadows: vec![
                    BoxShadow {
                        color: rgba_hex(0xFFFFFF, 0.26),
                        offset: point(px(0.0), px(1.0)),
                        blur_radius: px(1.5),
                        spread_radius: px(0.0),
                        inset: true,
                    },
                    BoxShadow {
                        color: rgba_hex(0x000000, 0.40),
                        offset: point(px(0.0), px(-1.5)),
                        blur_radius: px(3.0),
                        spread_radius: px(0.0),
                        inset: true,
                    },
                    BoxShadow {
                        color: rgba_hex(0x000000, 0.54),
                        offset: point(px(0.0), px(14.0)),
                        blur_radius: px(24.0),
                        spread_radius: px(-4.0),
                        inset: false,
                    },
                    BoxShadow {
                        color: rgba_hex(0x000000, 0.28),
                        offset: point(px(0.0), px(4.0)),
                        blur_radius: px(8.0),
                        spread_radius: px(0.0),
                        inset: false,
                    },
                ],
            }
        } else if hovered {
            CapsuleLiquidGlassStyle {
                bg: glass_gradient(rgba_hex(0x3B4863, 0.66), rgba_hex(0x1B2333, 0.52)),
                shadows: vec![
                    BoxShadow {
                        color: rgba_hex(0xBFE0FF, 0.34),
                        offset: point(px(0.0), px(1.0)),
                        blur_radius: px(2.0),
                        spread_radius: px(0.0),
                        inset: true,
                    },
                    BoxShadow {
                        color: rgba_hex(0x000000, 0.34),
                        offset: point(px(0.0), px(-1.0)),
                        blur_radius: px(2.5),
                        spread_radius: px(0.0),
                        inset: true,
                    },
                    BoxShadow {
                        color: rgba_hex(0x60A5FA, 0.24),
                        offset: point(px(0.0), px(0.0)),
                        blur_radius: px(8.0),
                        spread_radius: px(0.0),
                        inset: true,
                    },
                    BoxShadow {
                        color: rgba_hex(0x3B82F6, 0.28),
                        offset: point(px(0.0), px(0.0)),
                        blur_radius: px(14.0),
                        spread_radius: px(0.0),
                        inset: false,
                    },
                    BoxShadow {
                        color: rgba_hex(0x000000, 0.48),
                        offset: point(px(0.0), px(12.0)),
                        blur_radius: px(22.0),
                        spread_radius: px(-3.0),
                        inset: false,
                    },
                ],
            }
        } else if pulse > 0.01 {
            CapsuleLiquidGlassStyle {
                bg: glass_gradient(
                    rgba_hex(0x253D5C, 0.56 + 0.08 * pulse),
                    rgba_hex(0x142235, 0.44 + 0.08 * pulse),
                ),
                shadows: vec![
                    BoxShadow {
                        color: rgba_hex(0x38BDF8, 0.22 + 0.20 * pulse),
                        offset: point(px(0.0), px(1.0)),
                        blur_radius: px(3.0),
                        spread_radius: px(0.0),
                        inset: true,
                    },
                    BoxShadow {
                        color: rgba_hex(0x22C55E, 0.26 * pulse),
                        offset: point(px(0.0), px(-1.0)),
                        blur_radius: px(5.0),
                        spread_radius: px(0.0),
                        inset: true,
                    },
                    BoxShadow {
                        color: rgba_hex(0x38BDF8, 0.32 * pulse),
                        offset: point(px(0.0), px(0.0)),
                        blur_radius: px(14.0),
                        spread_radius: px(1.0),
                        inset: false,
                    },
                    BoxShadow {
                        color: rgba_hex(0x000000, 0.42),
                        offset: point(px(0.0), px(10.0)),
                        blur_radius: px(22.0),
                        spread_radius: px(-3.0),
                        inset: false,
                    },
                ],
            }
        } else {
            CapsuleLiquidGlassStyle {
                bg: glass_gradient(rgba_hex(0x2C3548, 0.56), rgba_hex(0x141822, 0.44)),
                shadows: vec![
                    BoxShadow {
                        color: rgba_hex(0xFFFFFF, 0.22),
                        offset: point(px(0.0), px(1.0)),
                        blur_radius: px(1.5),
                        spread_radius: px(0.0),
                        inset: true,
                    },
                    BoxShadow {
                        color: rgba_hex(0x000000, 0.34),
                        offset: point(px(0.0), px(-1.0)),
                        blur_radius: px(2.5),
                        spread_radius: px(0.0),
                        inset: true,
                    },
                    BoxShadow {
                        color: rgba_hex(0xFFFFFF, 0.08),
                        offset: point(px(1.0), px(0.0)),
                        blur_radius: px(1.0),
                        spread_radius: px(0.0),
                        inset: true,
                    },
                    BoxShadow {
                        color: rgba_hex(0x000000, 0.44),
                        offset: point(px(0.0), px(10.0)),
                        blur_radius: px(22.0),
                        spread_radius: px(-4.0),
                        inset: false,
                    },
                    BoxShadow {
                        color: rgba_hex(0x000000, 0.22),
                        offset: point(px(0.0), px(2.0)),
                        blur_radius: px(6.0),
                        spread_radius: px(0.0),
                        inset: false,
                    },
                ],
            }
        }
    } else if dragging {
        CapsuleLiquidGlassStyle {
            bg: glass_gradient(rgba_hex(0xFFFFFF, 0.84), rgba_hex(0xE6EEF9, 0.66)),
            shadows: vec![
                BoxShadow {
                    color: rgba_hex(0xFFFFFF, 0.92),
                    offset: point(px(0.0), px(1.0)),
                    blur_radius: px(1.5),
                    spread_radius: px(0.0),
                    inset: true,
                },
                BoxShadow {
                    color: rgba_hex(0x0F172A, 0.12),
                    offset: point(px(0.0), px(-1.5)),
                    blur_radius: px(3.0),
                    spread_radius: px(0.0),
                    inset: true,
                },
                BoxShadow {
                    color: rgba_hex(0x0F172A, 0.24),
                    offset: point(px(0.0), px(14.0)),
                    blur_radius: px(24.0),
                    spread_radius: px(-4.0),
                    inset: false,
                },
                BoxShadow {
                    color: rgba_hex(0x0F172A, 0.10),
                    offset: point(px(0.0), px(4.0)),
                    blur_radius: px(8.0),
                    spread_radius: px(0.0),
                    inset: false,
                },
            ],
        }
    } else if hovered {
        CapsuleLiquidGlassStyle {
            bg: glass_gradient(rgba_hex(0xFFFFFF, 0.82), rgba_hex(0xDCEAFF, 0.60)),
            shadows: vec![
                BoxShadow {
                    color: rgba_hex(0xFFFFFF, 0.96),
                    offset: point(px(0.0), px(1.0)),
                    blur_radius: px(2.0),
                    spread_radius: px(0.0),
                    inset: true,
                },
                BoxShadow {
                    color: rgba_hex(0x2563EB, 0.16),
                    offset: point(px(0.0), px(-1.0)),
                    blur_radius: px(4.0),
                    spread_radius: px(0.0),
                    inset: true,
                },
                BoxShadow {
                    color: rgba_hex(0x2563EB, 0.20),
                    offset: point(px(0.0), px(0.0)),
                    blur_radius: px(14.0),
                    spread_radius: px(0.0),
                    inset: false,
                },
                BoxShadow {
                    color: rgba_hex(0x0F172A, 0.18),
                    offset: point(px(0.0), px(12.0)),
                    blur_radius: px(22.0),
                    spread_radius: px(-3.0),
                    inset: false,
                },
            ],
        }
    } else if pulse > 0.01 {
        CapsuleLiquidGlassStyle {
            bg: glass_gradient(
                rgba_hex(0xF0F9FF, 0.72 + 0.10 * pulse),
                rgba_hex(0xDBEAFE, 0.48 + 0.12 * pulse),
            ),
            shadows: vec![
                BoxShadow {
                    color: rgba_hex(0xFFFFFF, 0.96),
                    offset: point(px(0.0), px(1.0)),
                    blur_radius: px(2.0),
                    spread_radius: px(0.0),
                    inset: true,
                },
                BoxShadow {
                    color: rgba_hex(0x2563EB, 0.22 * pulse),
                    offset: point(px(0.0), px(-1.0)),
                    blur_radius: px(5.0),
                    spread_radius: px(0.0),
                    inset: true,
                },
                BoxShadow {
                    color: rgba_hex(0x2563EB, 0.24 * pulse),
                    offset: point(px(0.0), px(0.0)),
                    blur_radius: px(14.0),
                    spread_radius: px(1.0),
                    inset: false,
                },
                BoxShadow {
                    color: rgba_hex(0x0F172A, 0.16),
                    offset: point(px(0.0), px(10.0)),
                    blur_radius: px(22.0),
                    spread_radius: px(-3.0),
                    inset: false,
                },
            ],
        }
    } else {
        CapsuleLiquidGlassStyle {
            bg: glass_gradient(rgba_hex(0xFFFFFF, 0.72), rgba_hex(0xDCE8FA, 0.48)),
            shadows: vec![
                BoxShadow {
                    color: rgba_hex(0xFFFFFF, 0.92),
                    offset: point(px(0.0), px(1.0)),
                    blur_radius: px(1.5),
                    spread_radius: px(0.0),
                    inset: true,
                },
                BoxShadow {
                    color: rgba_hex(0x0F172A, 0.10),
                    offset: point(px(0.0), px(-1.0)),
                    blur_radius: px(2.5),
                    spread_radius: px(0.0),
                    inset: true,
                },
                BoxShadow {
                    color: rgba_hex(0xFFFFFF, 0.45),
                    offset: point(px(1.0), px(0.0)),
                    blur_radius: px(1.0),
                    spread_radius: px(0.0),
                    inset: true,
                },
                BoxShadow {
                    color: rgba_hex(0x0F172A, 0.17),
                    offset: point(px(0.0), px(10.0)),
                    blur_radius: px(22.0),
                    spread_radius: px(-4.0),
                    inset: false,
                },
                BoxShadow {
                    color: rgba_hex(0x0F172A, 0.08),
                    offset: point(px(0.0), px(2.0)),
                    blur_radius: px(6.0),
                    spread_radius: px(0.0),
                    inset: false,
                },
            ],
        }
    }
}

/// Renders the `38x58` liquid optical glass mini-capsule with top specular sheen arc,
/// layered ClipboardX brand mark, hover spring scaling, blind-drag guard, and pulse dot.
pub fn render_idle_mini_capsule(
    icon_scale: f32,
    icon_opacity: f32,
    pulse_strength: f32,
    palette: &ThemePalette,
    on_expand: Rc<dyn Fn()>,
) -> AnyElement {
    let icon_sz = (20.0 * icon_scale).clamp(14.0, 24.0);
    let pulse = pulse_strength.clamp(0.0, 1.0);
    let sheen_top_alpha = if palette.is_dark { 0.18 } else { 0.65 };
    let sheen_bg = linear_gradient(
        180.0,
        linear_color_stop(rgba_hex(0xFFFFFF, sheen_top_alpha), 0.0),
        linear_color_stop(rgba_hex(0xFFFFFF, 0.0), 1.0),
    );
    let pulse_dot_color = if palette.is_dark {
        rgba_hex(0x38BDF8, pulse)
    } else {
        rgba_hex(0x2563EB, pulse)
    };
    let pulse_glow_color = if palette.is_dark {
        rgba_hex(0x38BDF8, 0.70 * pulse)
    } else {
        rgba_hex(0x2563EB, 0.70 * pulse)
    };

    div()
        .id("mini-capsule-idle")
        .relative()
        .cursor_pointer()
        .size_full()
        .rounded(px(999.0))
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, _| {
            reset_blind_drag_click_guard();
        })
        .on_mouse_up(gpui_kit::MouseButton::Left, move |_, _, _| {
            if !is_blind_dragging() && !was_blind_drag_consumed() {
                on_expand();
            }
        })
        .child(
            div()
                .absolute()
                .top(px(2.0))
                .left(px(5.5))
                .w(px(27.0))
                .h(px(17.0))
                .rounded(px(999.0))
                .bg(sheen_bg),
        )
        .child(
            div()
                .opacity(icon_opacity.clamp(0.0, 1.0))
                .flex()
                .items_center()
                .justify_center()
                .child(brand_capsule_mark(icon_sz, palette.is_dark)),
        )
        .when(pulse > 0.02, |el| {
            el.child(
                div()
                    .absolute()
                    .bottom(px(6.0))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .size(px(4.5))
                            .rounded(px(999.0))
                            .bg(pulse_dot_color)
                            .shadow(vec![BoxShadow {
                                color: pulse_glow_color,
                                offset: point(px(0.0), px(0.0)),
                                blur_radius: px(4.0),
                                spread_radius: px(0.0),
                                inset: false,
                            }]),
                    ),
            )
        })
        .into_any_element()
}
