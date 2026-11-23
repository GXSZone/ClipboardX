#![forbid(unsafe_code)]

#[cfg(not(target_os = "windows"))]
use crate::ffi;

/// Normalized desktop runtime environment classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopSessionKind {
    Windows,
    MacOs,
    LinuxX11,
    LinuxWayland,
}

/// Cross-platform session capability report surfaced cleanly in Settings / About.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopSessionDiagnostics {
    pub kind: DesktopSessionKind,
    pub display_label: &'static str,
    pub auto_paste_supported: bool,
    pub accessibility_granted: bool,
}

/// Inspects current OS display protocol and synthetic-input readiness.
pub fn collect_session_diagnostics() -> DesktopSessionDiagnostics {
    #[cfg(target_os = "windows")]
    {
        DesktopSessionDiagnostics {
            kind: DesktopSessionKind::Windows,
            display_label: "Windows Desktop (Win32)",
            auto_paste_supported: true,
            accessibility_granted: true,
        }
    }

    #[cfg(target_os = "macos")]
    {
        let trusted = ffi::is_accessibility_trusted();
        DesktopSessionDiagnostics {
            kind: DesktopSessionKind::MacOs,
            display_label: "macOS Cocoa",
            auto_paste_supported: trusted,
            accessibility_granted: trusted,
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if ffi::is_restricted_wayland() {
            DesktopSessionDiagnostics {
                kind: DesktopSessionKind::LinuxWayland,
                display_label: "Linux Wayland (Copy-Only)",
                auto_paste_supported: false,
                accessibility_granted: true,
            }
        } else {
            DesktopSessionDiagnostics {
                kind: DesktopSessionKind::LinuxX11,
                display_label: "Linux X11",
                auto_paste_supported: true,
                accessibility_granted: true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_valid_platform_diagnostics() {
        let diag = collect_session_diagnostics();
        assert!(!diag.display_label.is_empty());
    }
}
