//! Minimal isolated OS FFI layer. Every `unsafe` block must sit in the narrowest
//! possible scope and carry an explicit `// SAFETY:` invariant comment.

use clipboardx_model::{ForegroundWindowToken, PlatformError};

#[derive(Debug, Clone)]
pub struct ActiveWindowInfo {
    pub app_name: Option<String>,
    pub app_id: Option<String>,
}

/// Checks if the OS clipboard currently holds a confidential/password-manager marker.
pub fn has_confidential_clipboard_marker() -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_impl::has_confidential_marker()
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Queries metadata about the currently focused foreground application.
pub fn query_active_window_info() -> ActiveWindowInfo {
    #[cfg(target_os = "windows")]
    {
        windows_impl::active_window_info()
    }
    #[cfg(not(target_os = "windows"))]
    {
        ActiveWindowInfo {
            app_name: None,
            app_id: None,
        }
    }
}

/// Captures an opaque token representing the currently focused OS window.
pub fn capture_foreground_window() -> Option<ForegroundWindowToken> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::capture_foreground_hwnd()
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Restores input focus to a previously captured OS window token.
pub fn restore_foreground_window(token: ForegroundWindowToken) -> Result<(), PlatformError> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::restore_foreground_hwnd(token)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = token;
        Ok(())
    }
}

/// Polls whether the modifier keys specified in `hotkey_spec` are physically held down.
pub fn is_hotkey_modifier_held(hotkey_spec: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_impl::is_modifier_held(hotkey_spec)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = hotkey_spec;
        false
    }
}

/// Detects whether the current desktop session is a restricted Wayland compositor.
pub fn is_restricted_wayland() -> bool {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE")
                .map(|s| s.eq_ignore_ascii_case("wayland"))
                .unwrap_or(false)
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        false
    }
}

/// Eight-directional resize handle edge for borderless floating windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowResizeEdge {
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    TopLeft,
}

/// Initiates an OS-native borderless window drag on the currently active window.
pub fn begin_native_window_drag() {
    #[cfg(target_os = "windows")]
    {
        windows_impl::begin_window_drag();
    }
}

/// Initiates an OS-native borderless window resize from the specified edge/corner.
pub fn begin_native_window_resize(edge: WindowResizeEdge) {
    #[cfg(target_os = "windows")]
    {
        windows_impl::begin_window_resize(edge);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = edge;
    }
}

/// Records the active native window handle belonging to the current process for edge docking.
pub fn remember_own_window_hwnd() {
    #[cfg(target_os = "windows")]
    {
        windows_impl::remember_own_window_hwnd();
    }
}

/// Returns `true` when the currently focused OS foreground window belongs to this process.
pub fn is_own_process_foreground() -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_impl::is_own_process_foreground()
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Returns `Some(true/false)` indicating whether the OS cursor currently lies inside our
/// floating window bounds (works reliably even when the window is unfocused), or `None`
/// on platforms that rely on toolkit hover callbacks.
pub fn is_cursor_over_own_window() -> Option<bool> {
    #[cfg(target_os = "windows")]
    {
        Some(windows_impl::is_cursor_over_own_window())
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Updates the always-on-top z-order of the floating window without stealing focus.
pub fn set_own_window_topmost(topmost: bool) {
    #[cfg(target_os = "windows")]
    {
        windows_impl::set_own_window_topmost(topmost);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = topmost;
    }
}

/// Disables OS DWM non-client shadow/rounding and installs `WM_NCHITTEST` pass-through
/// for the transparent outer shadow margin (`phys_shadow_margin`) and corner exterior
/// (`phys_corner_radius`), enabling arbitrary anti-aliased custom-rounded shadows.
pub fn configure_custom_shadow_window(phys_shadow_margin: i32, phys_corner_radius: i32) {
    #[cfg(target_os = "windows")]
    {
        windows_impl::configure_custom_shadow_window(phys_shadow_margin, phys_corner_radius);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (phys_shadow_margin, phys_corner_radius);
    }
}

/// Disables OS DWM shadow/rounding on the currently active transient popup window (such as the
/// custom-rendered tray context menu) without touching the main panel's cached HWND, and positions
/// it cleanly beside the physical anchor `(phys_anchor_x, phys_anchor_y)` inside the monitor work area.
pub fn configure_transient_popup_window(
    phys_anchor_x: i32,
    phys_anchor_y: i32,
    phys_visual_w: i32,
    phys_visual_h: i32,
    phys_shadow_margin: i32,
) {
    #[cfg(target_os = "windows")]
    {
        windows_impl::configure_transient_popup_window(
            phys_anchor_x,
            phys_anchor_y,
            phys_visual_w,
            phys_visual_h,
            phys_shadow_margin,
        );
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (
            phys_anchor_x,
            phys_anchor_y,
            phys_visual_w,
            phys_visual_h,
            phys_shadow_margin,
        );
    }
}

/// Returns the current physical screen visual rect `(left, top, right, bottom)` of the main
/// floating window (excluding the transparent outer shadow margin).
pub fn own_window_visual_rect() -> Option<(i32, i32, i32, i32)> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::own_window_visual_rect()
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Returns the current OS cursor vertical coordinate in physical screen pixels.
pub fn cursor_screen_y() -> Option<i32> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::cursor_screen_y()
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Returns the current OS cursor `(x, y)` coordinates in physical screen pixels.
pub fn cursor_screen_pos() -> Option<(i32, i32)> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::cursor_screen_pos()
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Returns whether the primary (left) physical mouse button is currently held down.
pub fn is_left_mouse_button_down() -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_impl::is_left_mouse_button_down()
    }
    #[cfg(not(target_os = "windows"))]
    {
        true
    }
}

/// Moves the floating window's visual top-left corner directly to `(phys_visual_x, phys_visual_y)`
/// without altering size, z-order, or focus (`SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE`),
/// enabling 1:1 zero-latency blind dragging from any surface or item row.
pub fn move_own_window_visual_to(phys_visual_x: i32, phys_visual_y: i32) -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_impl::move_own_window_visual_to(phys_visual_x, phys_visual_y)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (phys_visual_x, phys_visual_y);
        false
    }
}

/// Inspects the current visual window bounds after a blind drag in capsule/peek mode,
/// snapping to the left or right monitor work-area edge when within `snap_threshold_phys`,
/// and returns the remembered capsule anchor `(edge_x, center_y, anchored_right)`.
pub fn snap_or_read_capsule_anchor(
    phys_margin: i32,
    snap_threshold_phys: i32,
) -> Option<(i32, i32, bool)> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::snap_or_read_capsule_anchor(phys_margin, snap_threshold_phys)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (phys_margin, snap_threshold_phys);
        None
    }
}

/// Positions and resizes the capsule or quick-paste peek card around a remembered anchor
/// `(edge_x, center_y, anchored_right)` (or docks flush right-center when `anchor` is `None`).
/// Returns the resolved anchor `(edge_x, center_y, anchored_right)`.
pub fn place_own_window_at_capsule_anchor(
    phys_visual_width: i32,
    phys_visual_height: i32,
    phys_margin: i32,
    anchor: Option<(i32, i32, bool)>,
) -> Option<(i32, i32, bool)> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::place_own_window_at_capsule_anchor(
            phys_visual_width,
            phys_visual_height,
            phys_margin,
            anchor,
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (phys_visual_width, phys_visual_height, phys_margin, anchor);
        None
    }
}

/// Configures and positions the non-activating, click-through image hover preview popup window
/// beside the main floating panel (`gap_phys = 8px` logical scaled, vertically centered on
/// `phys_row_center_y`). Keeps the popup window at a constant `(phys_canvas_w, phys_canvas_h)`
/// size so switching between aspect ratios never triggers DXGI `ResizeBuffers` or layout drift,
/// while aligning the inner card flush to `main_l - gap` (left) or `main_r + gap` (right).
/// Returns `true` if placed on the left, `false` if flipped right.
pub fn position_hover_preview_popup(
    phys_canvas_w: i32,
    phys_canvas_h: i32,
    phys_card_w: i32,
    phys_shadow_margin: i32,
    phys_row_center_y: i32,
    gap_phys: i32,
) -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_impl::position_hover_preview_popup(
            phys_canvas_w,
            phys_canvas_h,
            phys_card_w,
            phys_shadow_margin,
            phys_row_center_y,
            gap_phys,
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (
            phys_canvas_w,
            phys_canvas_h,
            phys_card_w,
            phys_shadow_margin,
            phys_row_center_y,
            gap_phys,
        );
        true
    }
}

/// Hides the cached hover preview popup window immediately (`SW_HIDE`) without destroying
/// its OS window or GPU swapchain so subsequent hovers display with sub-millisecond latency.
pub fn hide_hover_preview_popup() {
    #[cfg(target_os = "windows")]
    {
        windows_impl::hide_hover_preview_popup();
    }
}

/// Clears the cached hover preview popup HWND when the preview window is destroyed.
pub fn reset_hover_preview_popup_hwnd() {
    #[cfg(target_os = "windows")]
    {
        windows_impl::reset_hover_preview_popup_hwnd();
    }
}

/// Returns the current physical screen origin `(x, y)` of the floating window's *visual*
/// top-left corner (excluding the outer transparent shadow margin).
pub fn own_window_visual_origin() -> Option<(i32, i32)> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::own_window_visual_origin()
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Atomically resizes the floating window to `(phys_visual_width, phys_visual_height)` while
/// keeping the visual rectangle inside the current monitor's work area (`phys_edge_margin`).
/// Prefers `preferred_visual_origin` (or the current visual top-left corner when `None`) and
/// pulls left/up only as needed to prevent edge clipping. Returns the settled visual origin.
pub fn resize_own_window_clamped(
    phys_visual_width: i32,
    phys_visual_height: i32,
    preferred_visual_origin: Option<(i32, i32)>,
    phys_edge_margin: i32,
) -> Option<(i32, i32)> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::resize_own_window_clamped(
            phys_visual_width,
            phys_visual_height,
            preferred_visual_origin,
            phys_edge_margin,
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (
            phys_visual_width,
            phys_visual_height,
            preferred_visual_origin,
            phys_edge_margin,
        );
        None
    }
}

/// Lists deduplicated display/executable names of currently running desktop applications
/// with visible top-level windows, suitable for picking blacklist targets.
pub fn list_running_desktop_apps() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::list_running_desktop_apps()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Vec::new()
    }
}

/// Checks whether Accessibility permission is granted for synthetic input injection.
pub fn is_accessibility_trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_impl::is_accessibility_trusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Hints the OS to trim idle working-set pages while the app rests in capsule or tray standby.
pub fn trim_process_working_set() {
    #[cfg(target_os = "windows")]
    {
        windows_impl::trim_process_working_set();
    }
}

#[cfg(target_os = "macos")]
mod macos_impl {
    #![allow(unsafe_code)]

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    pub fn is_accessibility_trusted() -> bool {
        // SAFETY: `AXIsProcessTrusted` takes no arguments and returns a C boolean
        // indicating whether the current process has macOS Accessibility trust.
        unsafe { AXIsProcessTrusted() }
    }
}

#[cfg(target_os = "windows")]
mod windows_impl {
    #![allow(unsafe_code)]

    use std::path::Path;
    use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

    use clipboardx_model::{ForegroundWindowToken, PlatformError};
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Dwm::{
        DWM_WINDOW_CORNER_PREFERENCE, DWMNCRENDERINGPOLICY, DWMNCRP_DISABLED,
        DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
        DwmSetWindowAttribute,
    };
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, MonitorFromWindow,
    };
    use windows::Win32::System::DataExchange::{
        IsClipboardFormatAvailable, RegisterClipboardFormatW,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Memory::{
        SETPROCESSWORKINGSETSIZEEX_FLAGS, SetProcessWorkingSetSizeEx,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetActiveWindow, GetAsyncKeyState, ReleaseCapture, VK_CONTROL, VK_LBUTTON, VK_LWIN,
        VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, EnumWindows, GWL_EXSTYLE, GWLP_WNDPROC, GetClientRect,
        GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
        GetWindowThreadProcessId, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTLEFT,
        HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, HTTRANSPARENT, HWND_NOTOPMOST, HWND_TOPMOST,
        ICON_BIG, ICON_SMALL, IMAGE_ICON, IsIconic, IsWindow, IsWindowVisible, LR_DEFAULTSIZE,
        LR_SHARED, LoadImageW, MINMAXINFO, PostMessageW, SW_HIDE, SW_RESTORE, SWP_NOACTIVATE,
        SWP_NOCOPYBITS, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
        SendMessageW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow,
        WM_ERASEBKGND, WM_GETMINMAXINFO, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_SETICON, WNDPROC,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    };
    use windows::core::{BOOL, PCWSTR, PWSTR};

    use super::{ActiveWindowInfo, WindowResizeEdge};

    static OWN_WINDOW_HWND: AtomicIsize = AtomicIsize::new(0);
    static HOVER_PREVIEW_HWND: AtomicIsize = AtomicIsize::new(0);
    static CONFIGURED_PREVIEW_HWND: AtomicIsize = AtomicIsize::new(0);
    static SUBCLASSED_HWND: AtomicIsize = AtomicIsize::new(0);
    static ORIG_WNDPROC: AtomicIsize = AtomicIsize::new(0);
    static SHADOW_MARGIN_PHYS: AtomicI32 = AtomicI32::new(0);
    static CORNER_RADIUS_PHYS: AtomicI32 = AtomicI32::new(0);

    fn register_format_id(name_utf16: &[u16]) -> u32 {
        // SAFETY: `name_utf16` is a static null-terminated UTF-16 slice valid for the call.
        unsafe { RegisterClipboardFormatW(PCWSTR::from_raw(name_utf16.as_ptr())) }
    }

    pub fn has_confidential_marker() -> bool {
        // Standard Windows clipboard privacy format names (UTF-16 null-terminated).
        const EXCLUDE_MONITOR: &[u16] = &[
            b'E' as u16,
            b'x' as u16,
            b'c' as u16,
            b'l' as u16,
            b'u' as u16,
            b'd' as u16,
            b'e' as u16,
            b'C' as u16,
            b'l' as u16,
            b'i' as u16,
            b'p' as u16,
            b'b' as u16,
            b'o' as u16,
            b'a' as u16,
            b'r' as u16,
            b'd' as u16,
            b'C' as u16,
            b'o' as u16,
            b'n' as u16,
            b't' as u16,
            b'e' as u16,
            b'n' as u16,
            b't' as u16,
            b'F' as u16,
            b'r' as u16,
            b'o' as u16,
            b'm' as u16,
            b'M' as u16,
            b'o' as u16,
            b'n' as u16,
            b'i' as u16,
            b't' as u16,
            b'o' as u16,
            b'r' as u16,
            b'P' as u16,
            b'r' as u16,
            b'o' as u16,
            b'c' as u16,
            b'e' as u16,
            b's' as u16,
            b's' as u16,
            b'i' as u16,
            b'n' as u16,
            b'g' as u16,
            0,
        ];
        const VIEWER_IGNORE: &[u16] = &[
            b'C' as u16,
            b'l' as u16,
            b'i' as u16,
            b'p' as u16,
            b'b' as u16,
            b'o' as u16,
            b'a' as u16,
            b'r' as u16,
            b'd' as u16,
            b' ' as u16,
            b'V' as u16,
            b'i' as u16,
            b'e' as u16,
            b'w' as u16,
            b'e' as u16,
            b'r' as u16,
            b' ' as u16,
            b'I' as u16,
            b'g' as u16,
            b'n' as u16,
            b'o' as u16,
            b'r' as u16,
            b'e' as u16,
            0,
        ];

        for fmt in [
            register_format_id(EXCLUDE_MONITOR),
            register_format_id(VIEWER_IGNORE),
        ] {
            if fmt != 0 {
                // SAFETY: `IsClipboardFormatAvailable` takes an integer clipboard format ID
                // and performs a thread-safe OS query without opening the clipboard.
                let available = unsafe { IsClipboardFormatAvailable(fmt).is_ok() };
                if available {
                    return true;
                }
            }
        }
        false
    }

    pub fn active_window_info() -> ActiveWindowInfo {
        // SAFETY: `GetForegroundWindow` returns the current foreground HWND or null.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return ActiveWindowInfo {
                app_name: None,
                app_id: None,
            };
        }

        let mut pid: u32 = 0;
        // SAFETY: `hwnd` is a valid foreground handle and `pid` is a mutable stack u32.
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        if pid == 0 {
            return ActiveWindowInfo {
                app_name: None,
                app_id: None,
            };
        }

        // SAFETY: Requests read-only limited process information for `pid`.
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            return ActiveWindowInfo {
                app_name: None,
                app_id: None,
            };
        };

        let mut buf = vec![0u16; 512];
        let mut len = buf.len() as u32;
        // SAFETY: `handle` is an open process handle and `buf` is a writable UTF-16 buffer of `len`.
        let ok = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR::from_raw(buf.as_mut_ptr()),
                &mut len,
            )
            .is_ok()
        };
        // SAFETY: `handle` must be closed once queried.
        let _ = unsafe { CloseHandle(handle) };

        if !ok || len == 0 {
            return ActiveWindowInfo {
                app_name: None,
                app_id: None,
            };
        }

        let exe_path = String::from_utf16_lossy(&buf[..len as usize]);
        let app_name = Path::new(&exe_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_owned);

        ActiveWindowInfo {
            app_name,
            app_id: Some(exe_path),
        }
    }

    pub fn capture_foreground_hwnd() -> Option<ForegroundWindowToken> {
        // SAFETY: `GetForegroundWindow` safely returns the current foreground HWND.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            None
        } else {
            Some(ForegroundWindowToken(hwnd.0 as isize))
        }
    }

    pub fn restore_foreground_hwnd(token: ForegroundWindowToken) -> Result<(), PlatformError> {
        let hwnd = HWND(token.0 as *mut core::ffi::c_void);
        if hwnd.0.is_null() {
            return Err(PlatformError::Window("invalid null HWND token".to_owned()));
        }

        // SAFETY: `hwnd` came from a valid `GetForegroundWindow` call; Win32 handles stale
        // window handles safely by returning FALSE if the window no longer exists.
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            if !SetForegroundWindow(hwnd).as_bool() {
                return Err(PlatformError::Window(
                    "SetForegroundWindow returned false".to_owned(),
                ));
            }
        }
        Ok(())
    }

    pub fn is_modifier_held(hotkey_spec: &str) -> bool {
        let upper = hotkey_spec.to_ascii_uppercase();
        let mut required_vks = Vec::new();
        for part in upper.split('+') {
            match part.trim() {
                "ALT" | "OPTION" => required_vks.push(VK_MENU.0 as i32),
                "CTRL" | "CONTROL" => required_vks.push(VK_CONTROL.0 as i32),
                "SHIFT" => required_vks.push(VK_SHIFT.0 as i32),
                "WIN" | "SUPER" | "CMD" => {
                    // Either left or right Windows key satisfies Super.
                    let held = is_vk_down(VK_LWIN.0 as i32) || is_vk_down(VK_RWIN.0 as i32);
                    if !held {
                        return false;
                    }
                }
                _ => {}
            }
        }

        if required_vks.is_empty() && !upper.contains("WIN") && !upper.contains("SUPER") {
            return false;
        }
        required_vks.into_iter().all(is_vk_down)
    }

    fn is_vk_down(vk: i32) -> bool {
        // SAFETY: `GetAsyncKeyState` takes a Win32 virtual-key code and checks the MSB down bit.
        let state = unsafe { GetAsyncKeyState(vk) };
        (state as u16 & 0x8000) != 0
    }

    pub fn remember_own_window_hwnd() {
        // SAFETY: Queries the active/foreground window of the current process and validates
        // process ownership via `GetWindowThreadProcessId` before caching the HWND.
        unsafe {
            let mut hwnd = GetActiveWindow();
            if hwnd.0.is_null() {
                hwnd = GetForegroundWindow();
            }
            if !hwnd.0.is_null() {
                let raw = hwnd.0 as isize;
                if raw == HOVER_PREVIEW_HWND.load(Ordering::Acquire) {
                    return;
                }
                let mut pid: u32 = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid == std::process::id() {
                    OWN_WINDOW_HWND.store(raw, Ordering::Release);
                }
            }
        }
    }

    pub fn is_own_process_foreground() -> bool {
        // SAFETY: `GetForegroundWindow` and `GetWindowThreadProcessId` are read-only Win32 queries.
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() {
                return false;
            }
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let is_own = pid == std::process::id();
            if is_own && OWN_WINDOW_HWND.load(Ordering::Acquire) == 0 {
                let raw = hwnd.0 as isize;
                if raw != HOVER_PREVIEW_HWND.load(Ordering::Acquire) {
                    OWN_WINDOW_HWND.store(raw, Ordering::Release);
                }
            }
            is_own
        }
    }

    fn resolve_own_hwnd() -> Option<HWND> {
        let raw = OWN_WINDOW_HWND.load(Ordering::Acquire);
        if raw == 0 {
            remember_own_window_hwnd();
        }
        let raw = OWN_WINDOW_HWND.load(Ordering::Acquire);
        if raw == 0 {
            return None;
        }
        let hwnd = HWND(raw as *mut core::ffi::c_void);
        // SAFETY: `IsWindow` safely validates whether a cached HWND still refers to a live window.
        if unsafe { IsWindow(Some(hwnd)).as_bool() } {
            Some(hwnd)
        } else {
            OWN_WINDOW_HWND.store(0, Ordering::Release);
            None
        }
    }

    fn point_inside_visual_rounded_rect(rect: &RECT, screen_x: i32, screen_y: i32) -> bool {
        let margin = SHADOW_MARGIN_PHYS.load(Ordering::Acquire).max(0);
        let left = rect.left + margin;
        let top = rect.top + margin;
        let right = rect.right - margin;
        let bottom = rect.bottom - margin;
        if right <= left || bottom <= top {
            return true;
        }
        if screen_x < left || screen_x >= right || screen_y < top || screen_y >= bottom {
            return false;
        }
        let w = right - left;
        let h = bottom - top;
        let r = CORNER_RADIUS_PHYS
            .load(Ordering::Acquire)
            .clamp(0, (w / 2).min(h / 2));
        if r <= 0 {
            return true;
        }
        let lx = screen_x - left;
        let ly = screen_y - top;
        let dx = if lx < r {
            r - lx
        } else if lx >= w - r {
            lx - (w - r) + 1
        } else {
            0
        };
        let dy = if ly < r {
            r - ly
        } else if ly >= h - r {
            ly - (h - r) + 1
        } else {
            0
        };
        if dx == 0 || dy == 0 {
            return true;
        }
        (i64::from(dx) * i64::from(dx) + i64::from(dy) * i64::from(dy))
            <= (i64::from(r) * i64::from(r))
    }

    unsafe extern "system" fn shadow_passthrough_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_ERASEBKGND {
            // Suppress GDI background erase on our DirectX-rendered transparent window to prevent
            // white/black flashes and GDI-DWM synchronization stalls during resize/morph.
            return LRESULT(1);
        }
        if msg == WM_GETMINMAXINFO && lparam.0 != 0 {
            // SAFETY: For `WM_GETMINMAXINFO`, Win32 passes a valid mutable `MINMAXINFO` pointer in `lparam`.
            // Lowering `ptMinTrackSize` allows the borderless window to shrink down to the 38x58 mini-capsule
            // without passing `SWP_NOSENDCHANGING` (which would suppress `WM_WINDOWPOSCHANGING` / `WM_SIZE`).
            let mmi = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            mmi.ptMinTrackSize.x = 32;
            mmi.ptMinTrackSize.y = 32;
            return LRESULT(0);
        }
        if msg == WM_NCHITTEST {
            let sx = (lparam.0 & 0xFFFF) as i16 as i32;
            let sy = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut rect = RECT::default();
            // SAFETY: `GetWindowRect` is a read-only Win32 query on the live `hwnd`.
            if unsafe { GetWindowRect(hwnd, &mut rect).is_ok() }
                && !point_inside_visual_rounded_rect(&rect, sx, sy)
            {
                return LRESULT(HTTRANSPARENT as isize);
            }
        }
        let orig_ptr = ORIG_WNDPROC.load(Ordering::Acquire);
        if orig_ptr != 0 {
            // SAFETY: `orig_ptr` was returned by `SetWindowLongPtrW(hwnd, GWLP_WNDPROC, ...)`
            // for this exact window class and remains valid for the window's lifetime.
            let orig: WNDPROC = unsafe { std::mem::transmute(orig_ptr) };
            unsafe { CallWindowProcW(orig, hwnd, msg, wparam, lparam) }
        } else {
            // SAFETY: Fallback to standard Win32 default window procedure.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }

    pub fn configure_custom_shadow_window(phys_shadow_margin: i32, phys_corner_radius: i32) {
        SHADOW_MARGIN_PHYS.store(phys_shadow_margin.max(0), Ordering::Release);
        CORNER_RADIUS_PHYS.store(phys_corner_radius.max(0), Ordering::Release);
        let Some(hwnd) = resolve_own_hwnd() else {
            return;
        };
        let hwnd_raw = hwnd.0 as isize;
        if SUBCLASSED_HWND.load(Ordering::Acquire) == hwnd_raw {
            return;
        }

        // SAFETY: `hwnd` was validated by `IsWindow`; disables DWM non-client shadow and
        // system 8px rounding, then chains `GWLP_WNDPROC` once per HWND for shadow hit-test passthrough.
        unsafe {
            let nc_policy: DWMNCRENDERINGPOLICY = DWMNCRP_DISABLED;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_NCRENDERING_POLICY,
                (&nc_policy as *const DWMNCRENDERINGPOLICY).cast(),
                std::mem::size_of::<DWMNCRENDERINGPOLICY>() as u32,
            );
            let corner_pref: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_DONOTROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&corner_pref as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
                std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
            );
            let prev = SetWindowLongPtrW(
                hwnd,
                GWLP_WNDPROC,
                shadow_passthrough_wndproc as *const () as usize as isize,
            );
            if prev != 0 && prev != (shadow_passthrough_wndproc as *const () as usize as isize) {
                ORIG_WNDPROC.store(prev, Ordering::Release);
                SUBCLASSED_HWND.store(hwnd_raw, Ordering::Release);
            }

            // Bind the embedded exe icon resource (ID 1 generated by winresource) to the live HWND
            // so Alt+Tab and window switchers display the brand icon.
            if let Ok(hmod) = GetModuleHandleW(None)
                && let Ok(hicon) = LoadImageW(
                    Some(hmod.into()),
                    PCWSTR(std::ptr::without_provenance::<u16>(1)),
                    IMAGE_ICON,
                    0,
                    0,
                    LR_DEFAULTSIZE | LR_SHARED,
                )
            {
                let _ = SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_BIG as usize)),
                    Some(LPARAM(hicon.0 as isize)),
                );
                let _ = SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_SMALL as usize)),
                    Some(LPARAM(hicon.0 as isize)),
                );
            }
        }
    }

    pub fn configure_transient_popup_window(
        phys_anchor_x: i32,
        phys_anchor_y: i32,
        phys_visual_w: i32,
        phys_visual_h: i32,
        phys_shadow_margin: i32,
    ) {
        // SAFETY: Queries the freshly opened active popup HWND belonging to this process,
        // disables DWM non-client frame/rounding so custom GPUI shadows render cleanly, and
        // positions the popup beside `(phys_anchor_x, phys_anchor_y)` within the work area.
        unsafe {
            let hwnd = GetActiveWindow();
            if hwnd.0.is_null() {
                return;
            }
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid != std::process::id() {
                return;
            }

            let nc_policy: DWMNCRENDERINGPOLICY = DWMNCRP_DISABLED;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_NCRENDERING_POLICY,
                (&nc_policy as *const DWMNCRENDERINGPOLICY).cast(),
                std::mem::size_of::<DWMNCRENDERINGPOLICY>() as u32,
            );
            let corner_pref: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_DONOTROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&corner_pref as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
                std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
            );

            let shadow_pad = phys_shadow_margin.max(0);
            let (nc_w, nc_h) = non_client_size_delta(hwnd);
            let vis_w = phys_visual_w.max(120);
            let vis_h = phys_visual_h.max(80);
            let total_w = vis_w + shadow_pad * 2 + nc_w;
            let total_h = vis_h + shadow_pad * 2 + nc_h;

            let pt = POINT {
                x: phys_anchor_x,
                y: phys_anchor_y,
            };
            let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let (vis_x, vis_y) = if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let work = info.rcWork;
                let gap = 8;
                // Prefer opening upward above the bottom taskbar; fall back downward if near top.
                let preferred_y = if phys_anchor_y - vis_h - gap >= work.top {
                    phys_anchor_y - vis_h - gap
                } else {
                    phys_anchor_y + gap
                };
                let preferred_x = phys_anchor_x - vis_w / 2;
                let min_x = work.left + gap;
                let max_x = (work.right - vis_w - gap).max(min_x);
                let min_y = work.top + gap;
                let max_y = (work.bottom - vis_h - gap).max(min_y);
                (
                    preferred_x.clamp(min_x, max_x),
                    preferred_y.clamp(min_y, max_y),
                )
            } else {
                (phys_anchor_x - vis_w, phys_anchor_y - vis_h)
            };

            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                vis_x - shadow_pad,
                vis_y - shadow_pad,
                total_w,
                total_h,
                SWP_NOOWNERZORDER,
            );
        }
    }

    pub fn is_cursor_over_own_window() -> bool {
        let Some(hwnd) = resolve_own_hwnd() else {
            return false;
        };
        // SAFETY: `hwnd` was validated by `IsWindow`; `GetCursorPos` and `GetWindowRect`
        // are read-only Win32 queries populating stack structs.
        unsafe {
            let mut pt = POINT::default();
            let mut rect = RECT::default();
            if GetCursorPos(&mut pt).is_ok() && GetWindowRect(hwnd, &mut rect).is_ok() {
                point_inside_visual_rounded_rect(&rect, pt.x, pt.y)
            } else {
                false
            }
        }
    }

    pub fn own_window_visual_rect() -> Option<(i32, i32, i32, i32)> {
        let hwnd = resolve_own_hwnd()?;
        let shadow_pad = SHADOW_MARGIN_PHYS.load(Ordering::Acquire).max(0);
        let (nc_w, nc_h) = non_client_size_delta(hwnd);
        let mut rect = RECT::default();
        // SAFETY: `hwnd` was validated by `IsWindow`; `GetWindowRect` populates `rect`.
        if unsafe { GetWindowRect(hwnd, &mut rect).is_ok() } {
            Some((
                rect.left + shadow_pad,
                rect.top + shadow_pad,
                rect.right - shadow_pad - nc_w,
                rect.bottom - shadow_pad - nc_h,
            ))
        } else {
            None
        }
    }

    pub fn cursor_screen_y() -> Option<i32> {
        let mut pt = POINT::default();
        // SAFETY: `GetCursorPos` writes into a stack `POINT`.
        if unsafe { GetCursorPos(&mut pt).is_ok() } {
            Some(pt.y)
        } else {
            None
        }
    }

    pub fn cursor_screen_pos() -> Option<(i32, i32)> {
        let mut pt = POINT::default();
        // SAFETY: `GetCursorPos` writes physical screen cursor coordinates into a stack `POINT`.
        if unsafe { GetCursorPos(&mut pt).is_ok() } {
            Some((pt.x, pt.y))
        } else {
            None
        }
    }

    pub fn is_left_mouse_button_down() -> bool {
        is_vk_down(VK_LBUTTON.0 as i32)
    }

    pub fn move_own_window_visual_to(phys_visual_x: i32, phys_visual_y: i32) -> bool {
        let Some(hwnd) = resolve_own_hwnd() else {
            return false;
        };
        let shadow_pad = SHADOW_MARGIN_PHYS.load(Ordering::Acquire).max(0);
        // SAFETY: `hwnd` was validated by `IsWindow`; `SetWindowPos` translates the outer window
        // origin without altering size, z-order, or active focus (`SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE`).
        unsafe {
            SetWindowPos(
                hwnd,
                None,
                phys_visual_x - shadow_pad,
                phys_visual_y - shadow_pad,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
            .is_ok()
        }
    }

    pub fn snap_or_read_capsule_anchor(
        phys_margin: i32,
        snap_threshold_phys: i32,
    ) -> Option<(i32, i32, bool)> {
        let hwnd = resolve_own_hwnd()?;
        let shadow_pad = SHADOW_MARGIN_PHYS.load(Ordering::Acquire).max(0);
        let (nc_w, nc_h) = non_client_size_delta(hwnd);
        let edge_margin = phys_margin.max(0);
        let threshold = snap_threshold_phys.max(0);

        // SAFETY: `hwnd` was validated by `IsWindow`; queries current window rect and monitor work
        // area, snapping to the left or right edge if within `threshold` without stealing focus.
        unsafe {
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_err() {
                return None;
            }
            let cur_vis_x = rect.left + shadow_pad;
            let cur_vis_y = rect.top + shadow_pad;
            let vis_w = (rect.right - rect.left - shadow_pad * 2 - nc_w).max(24);
            let vis_h = (rect.bottom - rect.top - shadow_pad * 2 - nc_h).max(24);

            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return Some((cur_vis_x + vis_w, cur_vis_y + vis_h / 2, true));
            }
            let work = info.rcWork;
            let min_x = work.left + edge_margin;
            let max_x = (work.right - vis_w - edge_margin).max(min_x);
            let min_y = work.top + edge_margin;
            let max_y = (work.bottom - vis_h - edge_margin).max(min_y);

            let clamped_x = cur_vis_x.clamp(min_x, max_x);
            let clamped_y = cur_vis_y.clamp(min_y, max_y);

            let dist_left = (clamped_x - min_x).abs();
            let dist_right = (max_x - clamped_x).abs();
            let snapped_x = if dist_right <= threshold && dist_right <= dist_left {
                max_x
            } else if dist_left <= threshold {
                min_x
            } else {
                clamped_x
            };

            if snapped_x != cur_vis_x || clamped_y != cur_vis_y {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    snapped_x - shadow_pad,
                    clamped_y - shadow_pad,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
            }

            let mid_work_x = work.left + (work.right - work.left) / 2;
            let center_x = snapped_x + vis_w / 2;
            let anchored_right = center_x >= mid_work_x;
            let edge_x = if anchored_right {
                snapped_x + vis_w
            } else {
                snapped_x
            };
            let center_y = clamped_y + vis_h / 2;
            Some((edge_x, center_y, anchored_right))
        }
    }

    pub fn place_own_window_at_capsule_anchor(
        phys_visual_width: i32,
        phys_visual_height: i32,
        phys_margin: i32,
        anchor: Option<(i32, i32, bool)>,
    ) -> Option<(i32, i32, bool)> {
        let hwnd = resolve_own_hwnd()?;
        let shadow_pad = SHADOW_MARGIN_PHYS.load(Ordering::Acquire).max(0);
        let (nc_w, nc_h) = non_client_size_delta(hwnd);
        let vis_w = phys_visual_width.max(24);
        let vis_h = phys_visual_height.max(24);
        let total_w = vis_w + shadow_pad * 2 + nc_w;
        let total_h = vis_h + shadow_pad * 2 + nc_h;
        let edge_margin = phys_margin.max(0);

        // SAFETY: `hwnd` was validated by `IsWindow`; queries monitor work area and positions
        // the window around `anchor` (or flush right-center if `None`) with `SWP_NOACTIVATE`.
        unsafe {
            let monitor = if let Some((ax, ay, _)) = anchor {
                MonitorFromPoint(POINT { x: ax, y: ay }, MONITOR_DEFAULTTONEAREST)
            } else {
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)
            };
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return anchor;
            }
            let work = info.rcWork;
            let work_h = (work.bottom - work.top).max(vis_h);
            let min_x = work.left + edge_margin;
            let max_x = (work.right - vis_w - edge_margin).max(min_x);
            let min_y = work.top + edge_margin;
            let max_y = (work.bottom - vis_h - edge_margin).max(min_y);

            let (vis_x, vis_y, resolved_anchor) =
                if let Some((edge_x, center_y, anchored_right)) = anchor {
                    let raw_x = if anchored_right {
                        edge_x - vis_w
                    } else {
                        edge_x
                    };
                    let raw_y = center_y - vis_h / 2;
                    (
                        raw_x.clamp(min_x, max_x),
                        raw_y.clamp(min_y, max_y),
                        (edge_x, center_y, anchored_right),
                    )
                } else {
                    let default_x = max_x;
                    let default_y = (work.top + (work_h - vis_h) / 2).clamp(min_y, max_y);
                    (
                        default_x,
                        default_y,
                        (default_x + vis_w, default_y + vis_h / 2, true),
                    )
                };

            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                vis_x - shadow_pad,
                vis_y - shadow_pad,
                total_w,
                total_h,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOCOPYBITS,
            );
            Some(resolved_anchor)
        }
    }

    pub fn hide_hover_preview_popup() {
        let cached = HOVER_PREVIEW_HWND.load(Ordering::Acquire);
        if cached == 0 {
            return;
        }
        let hwnd = HWND(cached as *mut core::ffi::c_void);
        // SAFETY: `IsWindow` validates the cached HWND before hiding it via `ShowWindow(SW_HIDE)`.
        unsafe {
            if IsWindow(Some(hwnd)).as_bool() {
                let _ = ShowWindow(hwnd, SW_HIDE);
            } else {
                HOVER_PREVIEW_HWND.store(0, Ordering::Release);
            }
        }
    }

    pub fn reset_hover_preview_popup_hwnd() {
        HOVER_PREVIEW_HWND.store(0, Ordering::Release);
        CONFIGURED_PREVIEW_HWND.store(0, Ordering::Release);
    }

    fn resolve_hover_preview_hwnd() -> Option<HWND> {
        let cached = HOVER_PREVIEW_HWND.load(Ordering::Acquire);
        if cached != 0 {
            let hwnd = HWND(cached as *mut core::ffi::c_void);
            // SAFETY: `IsWindow` validates the cached HWND.
            if unsafe { IsWindow(Some(hwnd)).as_bool() } {
                return Some(hwnd);
            }
            HOVER_PREVIEW_HWND.store(0, Ordering::Release);
        }

        struct Finder {
            self_pid: u32,
            main_hwnd: isize,
            found: isize,
        }

        unsafe extern "system" fn enum_find(hwnd: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: `lparam` points to the stack-pinned `Finder` for the duration of `EnumWindows`.
            let finder = unsafe { &mut *(lparam.0 as *mut Finder) };
            let raw = hwnd.0 as isize;
            if raw == finder.main_hwnd {
                return BOOL(1);
            }
            // SAFETY: Read-only Win32 queries on `hwnd`.
            if !unsafe { IsWindowVisible(hwnd).as_bool() } {
                return BOOL(1);
            }
            let mut pid: u32 = 0;
            // SAFETY: `pid` is a valid stack mutable u32.
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
            }
            if pid == finder.self_pid {
                finder.found = raw;
                return BOOL(0);
            }
            BOOL(1)
        }

        let mut finder = Finder {
            self_pid: std::process::id(),
            main_hwnd: OWN_WINDOW_HWND.load(Ordering::Acquire),
            found: 0,
        };
        // SAFETY: `enum_find` runs synchronously while `finder` lives on the stack.
        let _ =
            unsafe { EnumWindows(Some(enum_find), LPARAM(&mut finder as *mut Finder as isize)) };
        if finder.found != 0 {
            HOVER_PREVIEW_HWND.store(finder.found, Ordering::Release);
            Some(HWND(finder.found as *mut core::ffi::c_void))
        } else {
            None
        }
    }

    pub fn position_hover_preview_popup(
        phys_canvas_w: i32,
        phys_canvas_h: i32,
        phys_card_w: i32,
        phys_shadow_margin: i32,
        phys_row_center_y: i32,
        gap_phys: i32,
    ) -> bool {
        let Some(preview_hwnd) = resolve_hover_preview_hwnd() else {
            return true;
        };
        let Some((main_l, main_t, main_r, main_b)) = own_window_visual_rect() else {
            return true;
        };
        let shadow_pad = phys_shadow_margin.max(0);
        let (nc_w, nc_h) = non_client_size_delta(preview_hwnd);
        let canvas_w = phys_canvas_w.max(24);
        let canvas_h = phys_canvas_h.max(24);
        let card_w = phys_card_w.clamp(24, canvas_w);
        let total_w = canvas_w + shadow_pad * 2 + nc_w;
        let total_h = canvas_h + shadow_pad * 2 + nc_h;
        let gap = gap_phys.max(0);

        // SAFETY: `preview_hwnd` was validated by `IsWindow`; disables DWM frame once per HWND,
        // applies non-activating click-through styles, and positions beside the main panel without focus steal.
        unsafe {
            let preview_raw = preview_hwnd.0 as isize;
            if CONFIGURED_PREVIEW_HWND.load(Ordering::Acquire) != preview_raw {
                let nc_policy: DWMNCRENDERINGPOLICY = DWMNCRP_DISABLED;
                let _ = DwmSetWindowAttribute(
                    preview_hwnd,
                    DWMWA_NCRENDERING_POLICY,
                    (&nc_policy as *const DWMNCRENDERINGPOLICY).cast(),
                    std::mem::size_of::<DWMNCRENDERINGPOLICY>() as u32,
                );
                let corner_pref: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_DONOTROUND;
                let _ = DwmSetWindowAttribute(
                    preview_hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    (&corner_pref as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
                    std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
                );
                let cur_ex = GetWindowLongPtrW(preview_hwnd, GWL_EXSTYLE);
                let desired_ex = cur_ex
                    | (WS_EX_NOACTIVATE.0 as isize)
                    | (WS_EX_TRANSPARENT.0 as isize)
                    | (WS_EX_TOOLWINDOW.0 as isize);
                if desired_ex != cur_ex {
                    let _ = SetWindowLongPtrW(preview_hwnd, GWL_EXSTYLE, desired_ex);
                }
                CONFIGURED_PREVIEW_HWND.store(preview_raw, Ordering::Release);
            }

            let main_hwnd = resolve_own_hwnd().unwrap_or(preview_hwnd);
            let monitor = MonitorFromWindow(main_hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let (vis_x, vis_y, on_left) = if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let work = info.rcWork;
                let fits_left = main_l - gap - card_w >= work.left + 4;
                let fits_right = main_r + gap + card_w <= work.right - 4;
                let on_left = fits_left || !fits_right;
                let x = if on_left {
                    // Card is right-aligned (`.justify_end()`) inside `canvas_w`, so placing the
                    // canvas at `main_l - gap - canvas_w` pins the card's right edge at `main_l - gap`.
                    main_l - gap - canvas_w
                } else {
                    // Card is left-aligned (`.justify_start()`) inside `canvas_w`, so placing the
                    // canvas at `main_r + gap` pins the card's left edge at `main_r + gap`.
                    main_r + gap
                };
                let anchor_y = phys_row_center_y.clamp(main_t, main_b);
                let min_y = work.top + 6;
                let max_y = (work.bottom - canvas_h - 6).max(min_y);
                let y = (anchor_y - canvas_h / 2).clamp(min_y, max_y);
                (x, y, on_left)
            } else {
                (
                    main_l - gap - canvas_w,
                    phys_row_center_y - canvas_h / 2,
                    true,
                )
            };

            let mut cur_rect = RECT::default();
            let same_size = GetWindowRect(preview_hwnd, &mut cur_rect).is_ok()
                && (cur_rect.right - cur_rect.left) == total_w
                && (cur_rect.bottom - cur_rect.top) == total_h;
            let mut flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW;
            if same_size {
                flags |= SWP_NOSIZE;
            }
            let _ = SetWindowPos(
                preview_hwnd,
                Some(HWND_TOPMOST),
                vis_x - shadow_pad,
                vis_y - shadow_pad,
                total_w,
                total_h,
                flags,
            );
            on_left
        }
    }

    pub fn own_window_visual_origin() -> Option<(i32, i32)> {
        let hwnd = resolve_own_hwnd()?;
        let shadow_pad = SHADOW_MARGIN_PHYS.load(Ordering::Acquire).max(0);
        let mut rect = RECT::default();
        // SAFETY: `hwnd` was validated by `IsWindow`; `GetWindowRect` populates `rect`.
        if unsafe { GetWindowRect(hwnd, &mut rect).is_ok() } {
            Some((rect.left + shadow_pad, rect.top + shadow_pad))
        } else {
            None
        }
    }

    /// Computes the non-client border delta `(extra_w, extra_h)` between the outer window rect
    /// and the GPUI client canvas rect so `SetWindowPos` always yields an exact client viewport.
    fn non_client_size_delta(hwnd: HWND) -> (i32, i32) {
        let mut wr = RECT::default();
        let mut cr = RECT::default();
        // SAFETY: `hwnd` was validated by `IsWindow`; both queries populate stack `RECT`s.
        if unsafe { GetWindowRect(hwnd, &mut wr).is_ok() && GetClientRect(hwnd, &mut cr).is_ok() } {
            let win_w = (wr.right - wr.left).max(0);
            let win_h = (wr.bottom - wr.top).max(0);
            let cli_w = (cr.right - cr.left).max(0);
            let cli_h = (cr.bottom - cr.top).max(0);
            ((win_w - cli_w).max(0), (win_h - cli_h).max(0))
        } else {
            (0, 0)
        }
    }

    pub fn resize_own_window_clamped(
        phys_visual_width: i32,
        phys_visual_height: i32,
        preferred_visual_origin: Option<(i32, i32)>,
        phys_edge_margin: i32,
    ) -> Option<(i32, i32)> {
        let hwnd = resolve_own_hwnd()?;
        let shadow_pad = SHADOW_MARGIN_PHYS.load(Ordering::Acquire).max(0);
        let (nc_w, nc_h) = non_client_size_delta(hwnd);
        let vis_w = phys_visual_width.max(24);
        let vis_h = phys_visual_height.max(24);
        let total_w = vis_w + shadow_pad * 2 + nc_w;
        let total_h = vis_h + shadow_pad * 2 + nc_h;
        let edge_margin = phys_edge_margin.max(0);

        // SAFETY: `hwnd` was validated by `IsWindow`; queries current monitor work area and
        // updates window bounds atomically via `SetWindowPos` without changing z-order or focus.
        unsafe {
            let mut current_rect = RECT::default();
            if GetWindowRect(hwnd, &mut current_rect).is_err() {
                return None;
            }
            let (base_x, base_y) = preferred_visual_origin.unwrap_or((
                current_rect.left + shadow_pad,
                current_rect.top + shadow_pad,
            ));

            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let (vis_x, vis_y) = if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let work = info.rcWork;
                let min_x = work.left + edge_margin;
                let max_x = (work.right - vis_w - edge_margin).max(min_x);
                let min_y = work.top + edge_margin;
                let max_y = (work.bottom - vis_h - edge_margin).max(min_y);
                (base_x.clamp(min_x, max_x), base_y.clamp(min_y, max_y))
            } else {
                (base_x, base_y)
            };

            let _ = SetWindowPos(
                hwnd,
                None,
                vis_x - shadow_pad,
                vis_y - shadow_pad,
                total_w,
                total_h,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOCOPYBITS,
            );
            Some((vis_x, vis_y))
        }
    }

    pub fn set_own_window_topmost(topmost: bool) {
        let Some(hwnd) = resolve_own_hwnd() else {
            return;
        };
        let z_insert = if topmost {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        };
        // SAFETY: `hwnd` was validated by `IsWindow`; flags preserve geometry and avoid focus steal.
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(z_insert),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
    }

    pub fn list_running_desktop_apps() -> Vec<String> {
        use std::collections::BTreeSet;

        struct Collector {
            self_pid: u32,
            names: BTreeSet<String>,
        }

        unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: `lparam` points to the stack-pinned `Collector` for the duration of `EnumWindows`.
            let collector = unsafe { &mut *(lparam.0 as *mut Collector) };
            // SAFETY: `IsWindowVisible` & `GetWindowThreadProcessId` are read-only queries on `hwnd`.
            if !unsafe { IsWindowVisible(hwnd).as_bool() } {
                return BOOL(1);
            }
            let mut pid: u32 = 0;
            // SAFETY: `pid` is a valid stack mutable u32.
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
            }
            if pid == 0 || pid == collector.self_pid {
                return BOOL(1);
            }
            // SAFETY: Request limited read-only query access to inspect executable name.
            let Ok(handle) =
                (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
            else {
                return BOOL(1);
            };
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            // SAFETY: `handle` is open and `buf` holds `len` writable UTF-16 code units.
            let ok = unsafe {
                QueryFullProcessImageNameW(
                    handle,
                    PROCESS_NAME_WIN32,
                    PWSTR::from_raw(buf.as_mut_ptr()),
                    &mut len,
                )
                .is_ok()
            };
            // SAFETY: Always close the opened process handle before returning.
            let _ = unsafe { CloseHandle(handle) };

            if ok && len > 0 {
                let exe_path = String::from_utf16_lossy(&buf[..len as usize]);
                if let Some(stem) = Path::new(&exe_path).file_stem().and_then(|s| s.to_str()) {
                    let trimmed = stem.trim();
                    if !trimmed.is_empty()
                        && !trimmed.eq_ignore_ascii_case("ApplicationFrameHost")
                        && !trimmed.eq_ignore_ascii_case("TextInputHost")
                        && !trimmed.eq_ignore_ascii_case("ShellExperienceHost")
                        && !trimmed.eq_ignore_ascii_case("explorer")
                    {
                        collector.names.insert(trimmed.to_owned());
                    }
                }
            }
            BOOL(1)
        }

        let mut collector = Collector {
            self_pid: std::process::id(),
            names: BTreeSet::new(),
        };
        // SAFETY: `enum_cb` runs synchronously within `EnumWindows` while `collector` stays pinned on stack.
        let _ = unsafe {
            EnumWindows(
                Some(enum_cb),
                LPARAM(&mut collector as *mut Collector as isize),
            )
        };
        collector.names.into_iter().collect()
    }

    pub fn begin_window_drag() {
        post_nc_lbutton_down(HTCAPTION);
    }

    pub fn trim_process_working_set() {
        // SAFETY: Passing `usize::MAX` (`-1`) for both minimum and maximum working set sizes
        // to `SetProcessWorkingSetSizeEx` on `GetCurrentProcess()` is the documented Win32 API
        // to cooperatively trim idle pages from the current process working set.
        unsafe {
            let _ = SetProcessWorkingSetSizeEx(
                GetCurrentProcess(),
                usize::MAX,
                usize::MAX,
                SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
            );
        }
    }

    pub fn begin_window_resize(edge: WindowResizeEdge) {
        let hit_test = match edge {
            WindowResizeEdge::Top => HTTOP,
            WindowResizeEdge::TopRight => HTTOPRIGHT,
            WindowResizeEdge::Right => HTRIGHT,
            WindowResizeEdge::BottomRight => HTBOTTOMRIGHT,
            WindowResizeEdge::Bottom => HTBOTTOM,
            WindowResizeEdge::BottomLeft => HTBOTTOMLEFT,
            WindowResizeEdge::Left => HTLEFT,
            WindowResizeEdge::TopLeft => HTTOPLEFT,
        };
        post_nc_lbutton_down(hit_test);
    }

    fn post_nc_lbutton_down(hit_test: u32) {
        let Some(hwnd) = resolve_own_hwnd() else {
            return;
        };
        // SAFETY: `hwnd` was validated by `resolve_own_hwnd`; `GetCursorPos` reads the cursor;
        // `ReleaseCapture` + `PostMessageW` queues `WM_NCLBUTTONDOWN` asynchronously so GPUI
        // releases its active `App` RefCell borrow before Win32 enters its modal move/resize loop.
        unsafe {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let packed_pos =
                (((pt.y as i16 as u16) as usize) << 16) | ((pt.x as i16 as u16) as usize);
            let _ = ReleaseCapture();
            let _ = PostMessageW(
                Some(hwnd),
                WM_NCLBUTTONDOWN,
                WPARAM(hit_test as usize),
                LPARAM(packed_pos as isize),
            );
        }
    }
}
