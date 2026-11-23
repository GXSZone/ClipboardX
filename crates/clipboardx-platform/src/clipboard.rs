#![forbid(unsafe_code)]

use std::io::Cursor;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clipboard_rs::common::RustImage;
use clipboard_rs::{
    Clipboard, ClipboardContext, ClipboardHandler, ClipboardWatcher, ClipboardWatcherContext,
    ContentFormat, RustImageData, WatcherShutdown,
};
use clipboardx_model::{
    CapturedClipboardEvent, ClipboardEventHandler, ClipboardPayload, PlatformError,
};
use image::ImageFormat;
use sha2::{Digest, Sha256};

use crate::ffi;
use crate::privacy::{CaptureGate, SelfWriteSuppressor};

/// Guard handle that stops the background clipboard watcher when dropped.
pub struct ClipboardWatcherHandle {
    shutdown: Option<WatcherShutdown>,
}

impl Drop for ClipboardWatcherHandle {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.stop();
        }
    }
}

struct HandlerAdapter {
    gate: CaptureGate,
    suppressor: SelfWriteSuppressor,
    on_event: ClipboardEventHandler,
}

impl ClipboardHandler for HandlerAdapter {
    fn on_clipboard_change(&mut self) {
        let Ok(ctx) = ClipboardContext::new() else {
            return;
        };
        let formats = ctx.available_formats().unwrap_or_default();
        let win_info = ffi::query_active_window_info();

        if self.gate.should_skip_capture(
            win_info.app_name.as_deref(),
            win_info.app_id.as_deref(),
            &formats,
        ) {
            return;
        }

        let Some((payload, hash)) = read_clipboard_payload(&ctx) else {
            return;
        };

        if self.suppressor.should_ignore(&hash) {
            return;
        }

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        (self.on_event)(CapturedClipboardEvent {
            payload,
            source_app_name: win_info.app_name,
            source_app_id: win_info.app_id,
            captured_at_ms: now_ms,
        });
    }
}

/// Starts the background OS clipboard watcher thread.
pub fn start_clipboard_watcher(
    gate: CaptureGate,
    suppressor: SelfWriteSuppressor,
    on_event: ClipboardEventHandler,
) -> Result<ClipboardWatcherHandle, PlatformError> {
    let mut watcher = ClipboardWatcherContext::new()
        .map_err(|err| PlatformError::Clipboard(format!("init clipboard watcher failed: {err}")))?;
    let shutdown = watcher.get_shutdown_channel();
    watcher.add_handler(HandlerAdapter {
        gate,
        suppressor,
        on_event,
    });

    thread::Builder::new()
        .name("clipboardx-watcher".to_owned())
        .spawn(move || {
            watcher.start_watch();
        })
        .map_err(|err| PlatformError::Clipboard(format!("spawn watcher thread failed: {err}")))?;

    Ok(ClipboardWatcherHandle {
        shutdown: Some(shutdown),
    })
}

pub fn read_text_snapshot() -> Result<Option<String>, PlatformError> {
    let ctx = ClipboardContext::new()
        .map_err(|err| PlatformError::Clipboard(format!("open clipboard failed: {err}")))?;
    if !ctx.has(ContentFormat::Text) {
        return Ok(None);
    }
    match ctx.get_text() {
        Ok(text) if !text.is_empty() => Ok(Some(text)),
        Ok(_) => Ok(None),
        Err(err) => Err(PlatformError::Clipboard(format!(
            "read text snapshot failed: {err}"
        ))),
    }
}

pub fn clear_clipboard() -> Result<(), PlatformError> {
    let ctx = ClipboardContext::new()
        .map_err(|err| PlatformError::Clipboard(format!("open clipboard failed: {err}")))?;
    ctx.clear()
        .map_err(|err| PlatformError::Clipboard(format!("clear clipboard failed: {err}")))
}

pub fn write_clipboard_payload(
    payload: &ClipboardPayload,
    suppressor: &SelfWriteSuppressor,
) -> Result<(), PlatformError> {
    let ctx = ClipboardContext::new()
        .map_err(|err| PlatformError::Clipboard(format!("open clipboard failed: {err}")))?;

    let hash = payload_hash(payload);
    suppressor.suppress_for(hash, Duration::from_millis(800));

    match payload {
        ClipboardPayload::Text { plain } => ctx
            .set_text(plain.clone())
            .map_err(|err| PlatformError::Clipboard(format!("set text failed: {err}")))?,
        ClipboardPayload::RichText { plain, html, rtf } => {
            let mut contents = vec![clipboard_rs::ClipboardContent::Text(plain.clone())];
            if let Some(h) = html {
                contents.push(clipboard_rs::ClipboardContent::Html(h.clone()));
            }
            if let Some(r) = rtf {
                contents.push(clipboard_rs::ClipboardContent::Rtf(r.clone()));
            }
            ctx.set(contents)
                .map_err(|err| PlatformError::Clipboard(format!("set rich text failed: {err}")))?;
        }
        ClipboardPayload::Image { png_bytes, .. } => {
            let img = RustImageData::from_bytes(png_bytes)
                .map_err(|err| PlatformError::Clipboard(format!("decode png failed: {err}")))?;
            ctx.set_image(img)
                .map_err(|err| PlatformError::Clipboard(format!("set image failed: {err}")))?;
        }
        ClipboardPayload::Files { paths } => {
            ctx.set_files(paths.clone())
                .map_err(|err| PlatformError::Clipboard(format!("set files failed: {err}")))?;
        }
    }

    Ok(())
}

fn read_clipboard_payload(ctx: &ClipboardContext) -> Option<(ClipboardPayload, String)> {
    // 1. Files take precedence when Explorer/Finder puts path lists + text representation.
    if ctx.has(ContentFormat::Files)
        && let Ok(files) = ctx.get_files()
        && !files.is_empty()
    {
        let payload = ClipboardPayload::Files { paths: files };
        let hash = payload_hash(&payload);
        return Some((payload, hash));
    }

    // 2. Image data.
    if ctx.has(ContentFormat::Image)
        && let Ok(img) = ctx.get_image()
        && !img.is_empty()
        && let Ok(dyn_img) = img.get_dynamic_image()
    {
        let width = dyn_img.width();
        let height = dyn_img.height();
        let mut cursor = Cursor::new(Vec::new());
        if dyn_img.write_to(&mut cursor, ImageFormat::Png).is_ok() {
            let png_bytes = cursor.into_inner();
            let payload = ClipboardPayload::Image {
                png_bytes,
                width,
                height,
            };
            let hash = payload_hash(&payload);
            return Some((payload, hash));
        }
    }

    // 3. Rich text (HTML / RTF) or Plain Text.
    let plain = if ctx.has(ContentFormat::Text) {
        ctx.get_text().ok()
    } else {
        None
    };
    let html = if ctx.has(ContentFormat::Html) {
        ctx.get_html().ok().filter(|s| !s.is_empty())
    } else {
        None
    };
    let rtf = if ctx.has(ContentFormat::Rtf) {
        ctx.get_rich_text().ok().filter(|s| !s.is_empty())
    } else {
        None
    };

    match (plain, html, rtf) {
        (Some(p), None, None) if !p.is_empty() => {
            let payload = ClipboardPayload::Text { plain: p };
            let hash = payload_hash(&payload);
            Some((payload, hash))
        }
        (p_opt, h_opt, r_opt) if h_opt.is_some() || r_opt.is_some() => {
            let plain = p_opt.unwrap_or_default();
            let payload = ClipboardPayload::RichText {
                plain,
                html: h_opt,
                rtf: r_opt,
            };
            let hash = payload_hash(&payload);
            Some((payload, hash))
        }
        _ => None,
    }
}

fn payload_hash(payload: &ClipboardPayload) -> String {
    let mut hasher = Sha256::new();
    match payload {
        ClipboardPayload::Text { plain } => {
            hasher.update(b"text:");
            hasher.update(plain.as_bytes());
        }
        ClipboardPayload::RichText { plain, html, rtf } => {
            hasher.update(b"rich:");
            hasher.update(plain.as_bytes());
            if let Some(h) = html {
                hasher.update(h.as_bytes());
            }
            if let Some(r) = rtf {
                hasher.update(r.as_bytes());
            }
        }
        ClipboardPayload::Image { png_bytes, .. } => {
            hasher.update(b"img:");
            hasher.update(png_bytes);
        }
        ClipboardPayload::Files { paths } => {
            hasher.update(b"files:");
            for p in paths {
                hasher.update(p.as_bytes());
                hasher.update(b"\n");
            }
        }
    }
    hex::encode(hasher.finalize())
}
