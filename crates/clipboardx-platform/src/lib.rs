//! Cross-platform desktop OS bridge for ClipboardX.
//!
//! All `unsafe` OS FFI calls are strictly isolated inside `mod ffi` with explicit
//! `// SAFETY:` invariants. All other modules forbid `unsafe_code`.
#![deny(unsafe_op_in_unsafe_fn)]

pub mod autofill;
pub mod bridge;
pub mod clipboard;
pub mod diagnostics;
pub mod ffi;
pub mod hotkey;
pub mod privacy;
pub mod startup;
pub mod tray;

pub use autofill::{AutofillAction, execute_account_autofill};
pub use bridge::DesktopPlatformBridge;
pub use clipboard::start_clipboard_watcher;
pub use diagnostics::{DesktopSessionDiagnostics, DesktopSessionKind, collect_session_diagnostics};
pub use ffi::{
    WindowResizeEdge, begin_native_window_drag, begin_native_window_resize,
    configure_custom_shadow_window, configure_transient_popup_window, cursor_screen_pos,
    cursor_screen_y, hide_hover_preview_popup, is_cursor_over_own_window,
    is_left_mouse_button_down, is_own_process_foreground, list_running_desktop_apps,
    move_own_window_visual_to, own_window_visual_origin, own_window_visual_rect,
    place_own_window_at_capsule_anchor, position_hover_preview_popup, remember_own_window_hwnd,
    reset_hover_preview_popup_hwnd, resize_own_window_clamped, set_own_window_topmost,
    snap_or_read_capsule_anchor, trim_process_working_set,
};
pub use hotkey::{HotkeyService, HotkeyTriggerStateMachine, parse_hotkey};
pub use privacy::{CaptureGate, SelfWriteSuppressor};
pub use startup::sync_launch_at_startup;
pub use tray::{TrayAction, TrayService};

/// Safely opens an `http://` or `https://` URL in the user's default OS web browser.
pub fn open_external_url(url: &str) -> Result<(), clipboardx_model::PlatformError> {
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err(clipboardx_model::PlatformError::Unsupported(
            "only http and https URLs can be opened externally".to_owned(),
        ));
    }

    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", trimmed]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(trimmed);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(trimmed);
        c
    };

    cmd.spawn().map_err(|err| {
        clipboardx_model::PlatformError::Unsupported(format!("open browser failed: {err}"))
    })?;
    Ok(())
}
