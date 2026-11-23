#![forbid(unsafe_code)]

use auto_launch::AutoLaunchBuilder;
use clipboardx_model::PlatformError;

/// Synchronizes OS launch-at-startup registration for the current ClipboardX executable.
pub fn sync_launch_at_startup(enabled: bool) -> Result<(), PlatformError> {
    let exe_path = std::env::current_exe().map_err(|err| {
        PlatformError::Unsupported(format!("resolve executable path failed: {err}"))
    })?;
    let exe_str = exe_path.to_string_lossy().into_owned();

    let launcher = AutoLaunchBuilder::new()
        .set_app_name("ClipboardX")
        .set_app_path(&exe_str)
        .set_args(&["--minimized"])
        .build()
        .map_err(|err| PlatformError::Unsupported(format!("build auto-launch failed: {err}")))?;

    if enabled {
        launcher.enable().map_err(|err| {
            PlatformError::Unsupported(format!("enable launch at startup failed: {err}"))
        })?;
    } else if launcher.is_enabled().unwrap_or(false) {
        launcher.disable().map_err(|err| {
            PlatformError::Unsupported(format!("disable launch at startup failed: {err}"))
        })?;
    }

    Ok(())
}
