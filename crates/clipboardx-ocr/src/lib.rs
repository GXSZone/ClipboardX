#![forbid(unsafe_code)]

//! Pluggable OCR subsystem for ClipboardX:
//! - Built-in OS engines (Windows WinRT `Windows.Media.Ocr`, macOS Vision)
//! - On-demand verified external JSON-RPC worker plugins with warm pool reclamation

pub mod installer;
pub mod plugin;

#[cfg(target_os = "macos")]
pub mod native_macos;
#[cfg(target_os = "windows")]
pub mod native_win;

use std::path::Path;
use std::sync::Arc;

use clipboardx_model::{OcrError, OcrOutput, OcrProvider};

pub use installer::{InstalledOcrPlugin, OcrPluginManager};
pub use plugin::{JsonRpcOcrProvider, OcrPluginManifest};

/// Fallback no-op native provider on platforms without built-in OS OCR APIs (e.g. Linux),
/// guiding users toward downloadable JSON-RPC plugins in Settings.
#[derive(Debug, Default)]
pub struct UnavailableNativeOcrProvider;

impl OcrProvider for UnavailableNativeOcrProvider {
    fn id(&self) -> &str {
        "native.unavailable"
    }

    fn display_name(&self) -> &str {
        "Native OS OCR Unavailable (Install Plugin)"
    }

    fn is_available(&self) -> bool {
        false
    }

    fn recognize_png(&self, _: &[u8], _: Option<&str>) -> Result<OcrOutput, OcrError> {
        Err(OcrError::Unavailable(
            "no built-in OS OCR on this platform; install an OCR plugin in Settings".to_owned(),
        ))
    }
}

/// Returns the platform's built-in zero-dependency native OCR provider.
pub fn native_provider() -> Arc<dyn OcrProvider> {
    #[cfg(target_os = "windows")]
    {
        Arc::new(native_win::WindowsNativeOcrProvider::new())
    }
    #[cfg(target_os = "macos")]
    {
        Arc::new(native_macos::MacosNativeOcrProvider::new())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Arc::new(UnavailableNativeOcrProvider)
    }
}

/// Resolves the active `OcrProvider` according to user settings, preferring `active_plugin`
/// if installed and falling back to the OS native provider.
pub fn resolve_active_provider(
    active_plugin: Option<&str>,
    plugins_dir: &Path,
) -> Arc<dyn OcrProvider> {
    if let Some(id) = active_plugin.filter(|s| !s.trim().is_empty()) {
        let mgr = OcrPluginManager::new(plugins_dir.to_path_buf());
        if let Ok(provider) = mgr.load_provider(id) {
            return Arc::new(provider);
        }
    }
    native_provider()
}
