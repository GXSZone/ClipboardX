#![forbid(unsafe_code)]
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod history;
mod icons;
mod settings;
mod shell;
mod state;
mod theme;
mod vault;

use std::io::{Read, Write};

use clipboardx_core::AppPaths;
use clipboardx_ocr::native_provider;
use gpui_kit::QuitMode;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

use crate::shell::ShellController;
use crate::state::AppRuntime;

rust_i18n::i18n!("../../locales", fallback = "zh-CN");

fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let paths = AppPaths::resolve_default().ok()?;
    let _ = std::fs::create_dir_all(&paths.logs_dir);
    let appender = tracing_appender::rolling::daily(&paths.logs_dir, "clipboardx.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(non_blocking))
        .try_init();

    Some(guard)
}

fn run_native_ocr_worker_and_exit() -> ! {
    let mut png_bytes = Vec::new();
    let exit_code = if std::io::stdin().read_to_end(&mut png_bytes).is_ok()
        && let Ok(output) = native_provider().recognize_png(&png_bytes, None)
    {
        let _ = std::io::stdout().write_all(output.full_text.as_bytes());
        let _ = std::io::stdout().flush();
        0
    } else {
        1
    };
    std::process::exit(exit_code);
}

fn main() {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--native-ocr-worker")) {
        run_native_ocr_worker_and_exit();
    }

    let _log_guard = init_logging();

    let runtime = match AppRuntime::bootstrap() {
        Ok(rt) => rt,
        Err(err) => {
            tracing::error!(error = %err, "ClipboardX runtime bootstrap failed");
            eprintln!("ClipboardX failed to initialize: {err}");
            return;
        }
    };

    tracing::info!("ClipboardX runtime initialized; starting GPUI event loop");

    gpui_kit::application()
        .with_assets(crate::icons::AppAssets)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            crate::theme::register_embedded_fonts(cx);
            ShellController::start(runtime, cx);
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn i18n_loads_both_locales_and_translates_keys() {
        let locales = rust_i18n::available_locales!();
        assert!(locales.iter().any(|l| l == "zh-CN"));
        assert!(locales.iter().any(|l| l == "en-US"));

        rust_i18n::set_locale("zh-CN");
        assert_ne!(rust_i18n::t!("search.placeholder"), "search.placeholder");
        assert_eq!(rust_i18n::t!("filter.all"), "全部");

        rust_i18n::set_locale("en-US");
        assert_eq!(rust_i18n::t!("filter.all"), "All");
        rust_i18n::set_locale("zh-CN");
    }
}
