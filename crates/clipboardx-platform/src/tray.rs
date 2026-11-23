#![forbid(unsafe_code)]

use clipboardx_model::PlatformError;
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// Actions triggered from the system tray icon or the custom GPUI tray menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    ShowOrHide,
    OpenMenu { x: i32, y: i32 },
    TogglePause,
    OpenSettings,
    Quit,
}

/// Manages the persistent minimalist desktop tray icon, routing left-click to direct panel
/// toggle and right-click to our custom borderless GPUI tray context menu.
pub struct TrayService {
    _tray: TrayIcon,
}

impl TrayService {
    pub fn create(
        _show_label: &str,
        _pause_label: &str,
        _settings_label: &str,
        _quit_label: &str,
    ) -> Result<Self, PlatformError> {
        let icon = build_brand_tray_icon()?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("ClipboardX")
            .with_icon(icon)
            .build()
            .map_err(|err| PlatformError::Tray(format!("create tray icon failed: {err}")))?;

        Ok(Self { _tray: tray })
    }

    pub fn set_paused_label(&self, _label: &str) {
        // The custom GPUI tray menu reads live pause status directly on render.
    }

    pub fn poll_action(&self) -> Option<TrayAction> {
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = ev
            {
                match button {
                    MouseButton::Left => return Some(TrayAction::ShowOrHide),
                    MouseButton::Right => {
                        return Some(TrayAction::OpenMenu {
                            x: position.x.round() as i32,
                            y: position.y.round() as i32,
                        });
                    }
                    _ => {}
                }
            }
        }
        None
    }
}

/// Generates a 4x4 SSAA anti-aliased 32x32 ClipboardX brand tray icon:
/// deep navy-to-electric-blue squircle, white clipboard card, blue clip bar, and dual-tone X.
fn build_brand_tray_icon() -> Result<Icon, PlatformError> {
    const SIZE: usize = 32;
    const SUB: usize = 4;
    let mut rgba = vec![0u8; SIZE * SIZE * 4];

    for py in 0..SIZE {
        for px in 0..SIZE {
            let mut r_acc = 0.0_f32;
            let mut g_acc = 0.0_f32;
            let mut b_acc = 0.0_f32;
            let mut a_acc = 0.0_f32;

            for sy in 0..SUB {
                for sx in 0..SUB {
                    let x = px as f32 + (sx as f32 + 0.5) / SUB as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SUB as f32;

                    // Outer squircle badge: [1..31] with 22.5% corner radius.
                    if !inside_rounded_rect(x, y, 1.0, 1.0, 30.0, 30.0, 6.8) {
                        continue;
                    }

                    let grad_t = (((x - 1.0) * 0.45 + (y - 1.0) * 0.55) / 30.0).clamp(0.0, 1.0);
                    let mut r = lerp_u8(0x1E, 0x1D, grad_t);
                    let mut g = lerp_u8(0x29, 0x4E, grad_t);
                    let mut b = lerp_u8(0x3B, 0xD8, grad_t);

                    // Subtle offset backing history card.
                    if inside_rounded_rect(x, y, 10.2, 8.2, 13.6, 16.8, 3.2) {
                        r = r * 0.72 + 147.0 * 0.28;
                        g = g * 0.72 + 197.0 * 0.28;
                        b = b * 0.72 + 253.0 * 0.28;
                    }

                    // Foreground white clipboard card.
                    if inside_rounded_rect(x, y, 8.0, 7.2, 14.6, 18.0, 3.4) {
                        let card_t = ((y - 7.2) / 18.0).clamp(0.0, 1.0);
                        r = lerp_u8(0xFF, 0xE2, card_t);
                        g = lerp_u8(0xFF, 0xEE, card_t);
                        b = lerp_u8(0xFF, 0xFF, card_t);

                        // Dual-tone diagonal X strokes centered inside the card.
                        let d_main = dist_to_segment(x, y, 12.1, 13.4, 18.5, 19.8);
                        let d_accent = dist_to_segment(x, y, 18.5, 13.4, 12.1, 19.8);
                        if d_accent <= 1.25 {
                            r = 0x38 as f32;
                            g = 0xBD as f32;
                            b = 0xF8 as f32;
                        }
                        if d_main <= 1.25 {
                            r = 0x25 as f32;
                            g = 0x63 as f32;
                            b = 0xEB as f32;
                        }
                    }

                    // Top blue clip bar overlapping the card top edge.
                    if inside_rounded_rect(x, y, 11.6, 5.2, 7.4, 3.5, 1.75) {
                        r = 0x3B as f32;
                        g = 0x82 as f32;
                        b = 0xF6 as f32;
                    }

                    r_acc += r;
                    g_acc += g;
                    b_acc += b;
                    a_acc += 255.0;
                }
            }

            let samples = (SUB * SUB) as f32;
            let idx = (py * SIZE + px) * 4;
            if a_acc > 0.0 {
                let cov = a_acc / (samples * 255.0);
                let inv_cov = 1.0 / (cov * samples);
                rgba[idx] = (r_acc * inv_cov).clamp(0.0, 255.0).round() as u8;
                rgba[idx + 1] = (g_acc * inv_cov).clamp(0.0, 255.0).round() as u8;
                rgba[idx + 2] = (b_acc * inv_cov).clamp(0.0, 255.0).round() as u8;
                rgba[idx + 3] = (a_acc / samples).clamp(0.0, 255.0).round() as u8;
            }
        }
    }

    Icon::from_rgba(rgba, SIZE as u32, SIZE as u32)
        .map_err(|err| PlatformError::Tray(format!("build tray icon pixels failed: {err}")))
}

fn lerp_u8(a: u8, b: u8, t: f32) -> f32 {
    let fa = f32::from(a);
    let fb = f32::from(b);
    fa + (fb - fa) * t
}

fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let abx = bx - ax;
    let aby = by - ay;
    let len_sq = abx * abx + aby * aby;
    let t = if len_sq <= f32::EPSILON {
        0.0
    } else {
        (((px - ax) * abx + (py - ay) * aby) / len_sq).clamp(0.0, 1.0)
    };
    let dx = px - (ax + abx * t);
    let dy = py - (ay + aby * t);
    (dx * dx + dy * dy).sqrt()
}

fn inside_rounded_rect(x: f32, y: f32, left: f32, top: f32, w: f32, h: f32, r: f32) -> bool {
    let right = left + w;
    let bottom = top + h;
    if x < left || x > right || y < top || y > bottom {
        return false;
    }
    let rr = r.min(w * 0.5).min(h * 0.5);
    let dx = if x < left + rr {
        (left + rr) - x
    } else if x > right - rr {
        x - (right - rr)
    } else {
        0.0
    };
    let dy = if y < top + rr {
        (top + rr) - y
    } else if y > bottom - rr {
        y - (bottom - rr)
    } else {
        0.0
    };
    dx * dx + dy * dy <= rr * rr
}
