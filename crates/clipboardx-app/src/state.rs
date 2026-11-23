use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use clipboardx_core::{
    AppPaths, CbxArchive, CbxExportStats, CbxImportStats, ClipboardService, ConfigStore,
    VaultService,
};
use clipboardx_model::{
    AppConfig, ClipItemId, ClipKind, ClipSummary, ClipboardPayload, ClipboardXError,
    ForegroundWindowToken, HotkeyEventKind, PasteActionMode, PlatformBridge, SecretStore,
    UpdateClient, UpdateReleaseInfo, VaultEntry,
};
use clipboardx_ocr::{
    InstalledOcrPlugin, OcrPluginManager, native_provider, resolve_active_provider,
};
use clipboardx_platform::clipboard::ClipboardWatcherHandle;
use clipboardx_platform::hotkey::TriggerOutcome;
use clipboardx_platform::{
    AutofillAction, CaptureGate, DesktopPlatformBridge, DesktopSessionDiagnostics, HotkeyService,
    HotkeyTriggerStateMachine, SelfWriteSuppressor, TrayAction, TrayService,
    collect_session_diagnostics, execute_account_autofill, start_clipboard_watcher,
    sync_launch_at_startup,
};
use clipboardx_storage::{BlobStore, HistoryStore, OsSecretStore, VaultStore, crypto::CryptoKeys};
use clipboardx_updater::{
    GitHubUpdateClient, apply_atomic_binary_replacement, can_self_update_directory,
    cleanup_stale_backup, current_platform_key, download_and_verify_release,
};
use gpui_kit::{Image, ImageFormat};
use rust_i18n::t;

/// Events sent from background listeners or settings window to the GPUI UI loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEvent {
    HistoryUpdated,
    ConfigUpdated,
    UpdateStateChanged,
}

/// Runtime state machine for zero-telemetry background & manual software updates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(UpdateReleaseInfo),
    Downloading(String),
    ReadyToRestart {
        version: String,
        staged_binary: PathBuf,
    },
    AppliedPendingRelaunch(String),
    Error(String),
}

/// Download & SHA-256 verification state machine for the optional RapidOCR ONNX plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum OcrDownloadStatus {
    #[default]
    Idle,
    Downloading {
        stage: String,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    Error(String),
}

/// Breakdown of local encrypted storage disk usage in bytes.
#[derive(Debug, Clone, Copy, Default)]
pub struct StorageBreakdown {
    pub db_bytes: u64,
    pub blobs_bytes: u64,
    pub thumbs_bytes: u64,
}

impl StorageBreakdown {
    pub fn total_bytes(self) -> u64 {
        self.db_bytes
            .saturating_add(self.blobs_bytes)
            .saturating_add(self.thumbs_bytes)
    }
}

const MAX_SHARED_IMAGE_CACHE: usize = 64;

/// Cached GPUI image handle plus original pixel dimensions shared across the list thumbnails,
/// right preview pane, capsule quick-paste card, and immersive hover preview popup so GPUI's
/// decoded GPU texture cache hits with zero disk I/O or decode flicker on hover.
#[derive(Clone)]
pub struct CachedImageThumb {
    pub image: Arc<Image>,
    pub orig_w: u32,
    pub orig_h: u32,
}

/// Shared application runtime assembling encrypted storage (`history.db` + `vault.db`),
/// core services, and cross-platform OS bridges.
pub struct AppRuntime {
    data_dir: PathBuf,
    history_db_path: PathBuf,
    vault_db_path: PathBuf,
    blobs_dir: PathBuf,
    thumbs_dir: PathBuf,
    updates_dir: PathBuf,
    config_store: ConfigStore,
    pub config: Arc<Mutex<AppConfig>>,
    pub service: ClipboardService,
    pub vault: VaultService,
    pub ocr_plugins: OcrPluginManager,
    ocr_tx: Sender<ClipItemId>,
    ocr_download_status: Arc<Mutex<OcrDownloadStatus>>,
    update_status: Arc<Mutex<UpdateStatus>>,
    pub platform: Arc<DesktopPlatformBridge>,
    pub suppressor: SelfWriteSuppressor,
    pub paused: Arc<AtomicBool>,
    pub hotkey_service: Mutex<Option<HotkeyService>>,
    pub trigger_sm: Mutex<HotkeyTriggerStateMachine>,
    pub tray: Mutex<Option<TrayService>>,
    pub last_foreground: Mutex<Option<ForegroundWindowToken>>,
    image_cache: RefCell<HashMap<ClipItemId, CachedImageThumb>>,
    _watcher: Mutex<Option<ClipboardWatcherHandle>>,
    ui_rx: Mutex<Receiver<UiEvent>>,
    ui_tx: Sender<UiEvent>,
}

impl AppRuntime {
    /// Initializes directories, master keys, encrypted history & vault stores,
    /// OS tray, global hotkeys, and background clipboard capture.
    pub fn bootstrap() -> Result<Rc<Self>, ClipboardXError> {
        let paths = AppPaths::resolve_default()?;
        ensure_runtime_dirs(&paths)?;

        if let Ok(current_exe) = std::env::current_exe() {
            cleanup_stale_backup(&current_exe);
        }

        let config_store = ConfigStore::new(paths.config_path.clone());
        let config = config_store.load_or_default()?;
        rust_i18n::set_locale(config.locale.code());

        let secret_store = Arc::new(OsSecretStore::new());
        let master_key = secret_store.get_or_create_master_key()?;
        let keys = CryptoKeys::derive_from_master(&master_key)?;

        let blobs = BlobStore::new(paths.blobs_dir.clone(), keys.blob_key)?;
        let store = HistoryStore::open(&paths.history_db_path, &keys.db_key_hex, blobs)?;
        let service = ClipboardService::new(store);

        let vault_store = VaultStore::new(paths.vault_db_path.clone());
        let vault = VaultService::new(vault_store, secret_store);
        if let Err(err) = vault.try_unlock_trusted(config.vault_unlock_policy) {
            tracing::warn!(error = %err, "trusted vault unlock skipped");
        }

        let suppressor = SelfWriteSuppressor::new();
        let platform = Arc::new(DesktopPlatformBridge::new(suppressor.clone()));
        let gate = CaptureGate::from_config(&config);
        let paused = Arc::new(AtomicBool::new(false));
        let (ui_tx, ui_rx) = mpsc::channel();
        let shared_config = Arc::new(Mutex::new(config));

        let plugins_dir = paths.plugins_dir.clone();
        let ocr_plugins = OcrPluginManager::new(plugins_dir.clone());
        let (ocr_tx, ocr_rx) = mpsc::channel::<ClipItemId>();
        spawn_background_ocr_worker(
            service.clone(),
            Arc::clone(&shared_config),
            plugins_dir.clone(),
            ocr_rx,
            ui_tx.clone(),
        );

        let watcher = {
            let service = service.clone();
            let config_arc = Arc::clone(&shared_config);
            let paused_flag = Arc::clone(&paused);
            let tx = ui_tx.clone();
            let ocr_queue = ocr_tx.clone();
            match start_clipboard_watcher(
                gate,
                suppressor.clone(),
                Box::new(move |event| {
                    if paused_flag.load(Ordering::Acquire) {
                        return;
                    }
                    let cfg_snap = config_arc
                        .lock()
                        .map(|c| c.clone())
                        .unwrap_or_else(|_| AppConfig::default());
                    let is_image = event.payload.kind() == ClipKind::Image;
                    match service.ingest_event(event, &cfg_snap) {
                        Ok(Some(id)) => {
                            if is_image && cfg_snap.auto_ocr_native {
                                let _ = ocr_queue.send(id);
                            }
                            let _ = tx.send(UiEvent::HistoryUpdated);
                        }
                        Ok(None) => {}
                        Err(err) => {
                            tracing::warn!(error = %err, "clipboard event ingestion failed");
                        }
                    }
                }),
            ) {
                Ok(handle) => Some(handle),
                Err(err) => {
                    tracing::warn!(error = %err, "clipboard watcher initialization failed");
                    None
                }
            }
        };

        let hotkey_cfg = shared_config
            .lock()
            .map(|c| c.clone())
            .unwrap_or_else(|_| AppConfig::default());
        let hotkey_service = match HotkeyService::register(
            &hotkey_cfg.toggle_hotkey,
            &hotkey_cfg.compact_hotkey,
            &hotkey_cfg.pause_hotkey,
        ) {
            Ok(svc) => Some(svc),
            Err(err) => {
                tracing::warn!(error = %err, "global hotkey registration partially unavailable");
                None
            }
        };

        let tray = match TrayService::create(
            &t!("tray.show_hide"),
            &t!("tray.pause_manual"),
            &t!("tray.settings"),
            &t!("tray.quit"),
        ) {
            Ok(svc) => Some(svc),
            Err(err) => {
                tracing::warn!(error = %err, "system tray initialization failed");
                None
            }
        };

        let ocr_download_status = Arc::new(Mutex::new(OcrDownloadStatus::Idle));
        let update_status = Arc::new(Mutex::new(UpdateStatus::Idle));
        let updates_dir = paths.updates_dir.clone();

        if hotkey_cfg.background_auto_update {
            spawn_startup_update_check(
                hotkey_cfg.update_channel,
                Arc::clone(&update_status),
                ui_tx.clone(),
            );
        }

        let thumbs_dir = paths.data_dir.join("thumbs");
        Ok(Rc::new(Self {
            data_dir: paths.data_dir,
            history_db_path: paths.history_db_path,
            vault_db_path: paths.vault_db_path,
            blobs_dir: paths.blobs_dir,
            thumbs_dir,
            updates_dir,
            config_store,
            config: shared_config,
            service,
            vault,
            ocr_plugins,
            ocr_tx,
            ocr_download_status,
            update_status,
            platform,
            suppressor,
            paused,
            hotkey_service: Mutex::new(hotkey_service),
            trigger_sm: Mutex::new(HotkeyTriggerStateMachine::new()),
            tray: Mutex::new(tray),
            last_foreground: Mutex::new(None),
            image_cache: RefCell::new(HashMap::new()),
            _watcher: Mutex::new(watcher),
            ui_rx: Mutex::new(ui_rx),
            ui_tx,
        }))
    }

    pub fn notify_ui(&self, ev: UiEvent) {
        let _ = self.ui_tx.send(ev);
    }

    /// Returns an already warmed image thumbnail from the shared UI-thread cache.
    pub fn cached_image_thumb(&self, id: ClipItemId) -> Option<CachedImageThumb> {
        self.image_cache.borrow().get(&id).cloned()
    }

    /// Loads and caches an image clip's thumbnail/blob handle from a summary row so list icons,
    /// right preview pane, and hover popups reuse the exact same decoded GPUI `Arc<Image>`.
    pub fn warm_image_thumb_from_summary(&self, summary: &ClipSummary) -> Option<CachedImageThumb> {
        if summary.kind != ClipKind::Image {
            return None;
        }
        if let Some(hit) = self.cached_image_thumb(summary.id) {
            return Some(hit);
        }
        let meta: serde_json::Value = serde_json::from_str(&summary.meta_json).unwrap_or_default();
        let orig_w = meta["width"].as_u64().unwrap_or(0) as u32;
        let orig_h = meta["height"].as_u64().unwrap_or(0) as u32;

        let blobs = self.service.store().blobs();
        let bytes = summary
            .thumb_ref
            .as_deref()
            .and_then(|r| blobs.get_bytes(r).ok())
            .or_else(|| {
                self.service
                    .get_item(summary.id)
                    .ok()
                    .and_then(|it| it.blob_ref)
                    .and_then(|r| blobs.get_bytes(&r).ok())
            })?;
        let entry = CachedImageThumb {
            image: Arc::new(Image::from_bytes(ImageFormat::Png, bytes)),
            orig_w,
            orig_h,
        };
        let mut cache = self.image_cache.borrow_mut();
        if cache.len() >= MAX_SHARED_IMAGE_CACHE {
            cache.clear();
        }
        cache.insert(summary.id, entry.clone());
        Some(entry)
    }

    /// Resolves a cached or freshly decrypted `CachedImageThumb` by `ClipItemId`.
    pub fn get_or_load_image_thumb(&self, id: ClipItemId) -> Option<CachedImageThumb> {
        if let Some(hit) = self.cached_image_thumb(id) {
            return Some(hit);
        }
        let item = self.service.get_item(id).ok()?;
        if item.summary.kind != ClipKind::Image {
            return None;
        }
        let meta: serde_json::Value =
            serde_json::from_str(&item.summary.meta_json).unwrap_or_default();
        let orig_w = meta["width"].as_u64().unwrap_or(0) as u32;
        let orig_h = meta["height"].as_u64().unwrap_or(0) as u32;

        let blobs = self.service.store().blobs();
        let bytes = item
            .summary
            .thumb_ref
            .as_deref()
            .and_then(|r| blobs.get_bytes(r).ok())
            .or_else(|| {
                item.blob_ref
                    .as_deref()
                    .and_then(|r| blobs.get_bytes(r).ok())
            })?;
        let entry = CachedImageThumb {
            image: Arc::new(Image::from_bytes(ImageFormat::Png, bytes)),
            orig_w,
            orig_h,
        };
        let mut cache = self.image_cache.borrow_mut();
        if cache.len() >= MAX_SHARED_IMAGE_CACHE {
            cache.clear();
        }
        cache.insert(id, entry.clone());
        Some(entry)
    }

    pub fn drain_ui_events(&self) -> Vec<UiEvent> {
        let mut events = Vec::new();
        if let Ok(rx) = self.ui_rx.lock() {
            while let Ok(ev) = rx.try_recv() {
                events.push(ev);
            }
        }
        events
    }

    pub fn poll_tray_action(&self) -> Option<TrayAction> {
        let guard = self.tray.lock().ok()?;
        guard.as_ref()?.poll_action()
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub fn toggle_pause(&self) -> bool {
        let next = !self.paused.load(Ordering::Acquire);
        self.paused.store(next, Ordering::Release);
        if let Ok(guard) = self.tray.lock()
            && let Some(tray) = guard.as_ref()
        {
            let label = if next {
                t!("tray.resume")
            } else {
                t!("tray.pause_manual")
            };
            tray.set_paused_label(&label);
        }
        next
    }

    pub fn snapshot_config(&self) -> AppConfig {
        self.config
            .lock()
            .map(|c| c.clone())
            .unwrap_or_else(|_| AppConfig::default())
    }

    /// Mutates persistent settings atomically, updates runtime locale/autostart if changed,
    /// saves `config.toml` asynchronously off the UI thread, and notifies open UI windows to refresh.
    pub fn update_config(&self, mutator: impl FnOnce(&mut AppConfig)) -> AppConfig {
        let mut prev_locale = clipboardx_model::Locale::ZhCn;
        let mut prev_startup = false;
        let snapshot = if let Ok(mut cfg) = self.config.lock() {
            prev_locale = cfg.locale;
            prev_startup = cfg.launch_at_startup;
            mutator(&mut cfg);
            cfg.clone()
        } else {
            AppConfig::default()
        };

        if snapshot.locale != prev_locale {
            rust_i18n::set_locale(snapshot.locale.code());
        }
        self.notify_ui(UiEvent::ConfigUpdated);

        let store = self.config_store.clone();
        let to_save = snapshot.clone();
        let sync_startup =
            (snapshot.launch_at_startup != prev_startup).then_some(snapshot.launch_at_startup);
        thread::spawn(move || {
            let _ = store.save(&to_save);
            if let Some(enabled) = sync_startup
                && let Err(err) = sync_launch_at_startup(enabled)
            {
                tracing::warn!(error = %err, "launch-at-startup synchronization failed");
            }
        });
        snapshot
    }

    pub fn set_compact_mode(&self, compact: bool) {
        let to_save = if let Ok(mut cfg) = self.config.lock() {
            if cfg.is_compact_mode == compact {
                return;
            }
            cfg.is_compact_mode = compact;
            cfg.remember_compact_mode.then(|| cfg.clone())
        } else {
            None
        };
        if let Some(cfg_snap) = to_save {
            let store = self.config_store.clone();
            thread::spawn(move || {
                let _ = store.save(&cfg_snap);
            });
        }
    }

    pub fn remember_foreground_window(&self) {
        if clipboardx_platform::is_own_process_foreground() {
            return;
        }
        let token = self.platform.capture_foreground_window();
        if token.is_some()
            && let Ok(mut guard) = self.last_foreground.lock()
        {
            *guard = token;
        }
    }

    /// Writes `id`'s payload to the OS clipboard and triggers focus restoration + paste
    /// if configured and not running on a restricted Wayland compositor.
    pub fn execute_paste(
        &self,
        id: ClipItemId,
        plain_text_only: bool,
        copy_only: bool,
    ) -> Result<(), ClipboardXError> {
        let payload = self.service.load_payload_for_paste(id, plain_text_only)?;
        self.platform.write_clipboard(&payload)?;

        let cfg = self.snapshot_config();
        let should_auto_paste = !copy_only
            && cfg.paste_mode == PasteActionMode::AutoPaste
            && !self.platform.is_restricted_wayland();

        if should_auto_paste {
            let token = self
                .last_foreground
                .lock()
                .ok()
                .and_then(|mut guard| guard.take());
            let platform = Arc::clone(&self.platform);
            thread::spawn(move || {
                if let Some(target) = token {
                    let _ = platform.restore_foreground_window(target);
                }
                // Brief settle delay allows the target window to become active before Ctrl+V.
                thread::sleep(Duration::from_millis(65));
                if let Err(err) = platform.simulate_paste() {
                    tracing::warn!(error = %err, "auto-paste simulation failed");
                }
            });
        }

        Ok(())
    }

    /// Runs vault credential autofill (or single-field paste) off the GPUI render thread
    /// with RAII prior-clipboard snapshot restoration.
    pub fn execute_vault_autofill(&self, entry: VaultEntry, action: AutofillAction) {
        let cfg = self.snapshot_config();
        let restore = cfg.restore_clipboard_after_autofill;
        let token = self
            .last_foreground
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());
        let platform = Arc::clone(&self.platform);
        let suppressor = self.suppressor.clone();

        thread::spawn(move || {
            if let Err(err) = execute_account_autofill(
                platform.as_ref(),
                &suppressor,
                token,
                &entry,
                action,
                restore,
            ) {
                tracing::warn!(error = %err, "vault autofill failed");
            }
        });
    }

    /// Consumes and translates global hotkey state transitions cleanly.
    pub fn handle_hotkey_tick(&self) -> (Option<HotkeyEventKind>, TriggerOutcome) {
        let cfg = self.snapshot_config();
        let ev = self
            .hotkey_service
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().and_then(|svc| svc.poll_event()));

        let toggle_outcome = if matches!(ev, Some(HotkeyEventKind::TogglePanelPressed)) {
            self.trigger_sm
                .lock()
                .map(|mut sm| {
                    sm.on_hotkey_pressed(cfg.hotkey_trigger_mode, std::time::Instant::now())
                })
                .unwrap_or(TriggerOutcome::None)
        } else {
            TriggerOutcome::None
        };

        (ev, toggle_outcome)
    }

    /// Default directory under user data directory for `.cbx` offline backup dialogs.
    pub fn backup_dir(&self) -> PathBuf {
        let dir = self.data_dir.join("backups");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// Computes real local disk usage across encrypted databases, blobs, and thumbnails.
    pub fn storage_breakdown(&self) -> StorageBreakdown {
        let db_bytes =
            file_len(&self.history_db_path).saturating_add(file_len(&self.vault_db_path));
        let blobs_bytes = dir_size(&self.blobs_dir);
        let thumbs_bytes = dir_size(&self.thumbs_dir);
        StorageBreakdown {
            db_bytes,
            blobs_bytes,
            thumbs_bytes,
        }
    }

    pub fn export_cbx(
        &self,
        path: &Path,
        passphrase: Option<&[u8]>,
    ) -> Result<CbxExportStats, ClipboardXError> {
        let cfg = self.snapshot_config();
        let vault_opt = self
            .vault
            .is_unlocked(cfg.vault_unlock_policy, cfg.vault_timeout_minutes)
            .then_some(&self.vault);
        CbxArchive::export_to_path(path, self.service.store(), vault_opt, passphrase)
    }

    pub fn import_cbx(
        &self,
        path: &Path,
        passphrase: Option<&[u8]>,
    ) -> Result<CbxImportStats, ClipboardXError> {
        let cfg = self.snapshot_config();
        let vault_opt = self
            .vault
            .is_unlocked(cfg.vault_unlock_policy, cfg.vault_timeout_minutes)
            .then_some(&self.vault);
        let stats =
            CbxArchive::import_from_path(path, self.service.store(), vault_opt, passphrase)?;
        self.notify_ui(UiEvent::HistoryUpdated);
        Ok(stats)
    }

    pub fn trigger_ocr_now(&self, id: ClipItemId) {
        let _ = self.ocr_tx.send(id);
    }

    /// Short label for the active native platform OCR engine (`WinRT` / `Vision` / `Native`).
    pub fn native_ocr_short_name(&self) -> &'static str {
        let provider = native_provider();
        let full = provider.display_name();
        if full.contains("WinRT") {
            "WinRT"
        } else if full.contains("Vision") {
            "Vision"
        } else {
            "Native"
        }
    }

    pub fn list_ocr_plugins(&self) -> Vec<InstalledOcrPlugin> {
        self.ocr_plugins.list_installed().unwrap_or_default()
    }

    pub fn install_ocr_plugin(
        &self,
        source: &str,
        expected_sha256: Option<&str>,
    ) -> Result<InstalledOcrPlugin, ClipboardXError> {
        let trimmed = source.trim();
        let installed = if trimmed.to_ascii_lowercase().starts_with("https://") {
            let sha = expected_sha256.unwrap_or("").trim();
            self.ocr_plugins.install_from_url(trimmed, sha)?
        } else {
            self.ocr_plugins
                .install_from_local_zip(Path::new(trimmed), expected_sha256)?
        };
        let plugin_id = installed.manifest.id.clone();
        self.update_config(|c| c.active_ocr_plugin = Some(plugin_id));
        Ok(installed)
    }

    pub fn uninstall_ocr_plugin(&self, plugin_id: &str) -> Result<(), ClipboardXError> {
        self.ocr_plugins.uninstall(plugin_id)?;
        let cfg = self.snapshot_config();
        if cfg.active_ocr_plugin.as_deref() == Some(plugin_id) {
            self.update_config(|c| c.active_ocr_plugin = None);
        }
        Ok(())
    }

    pub fn ocr_download_status(&self) -> OcrDownloadStatus {
        self.ocr_download_status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    pub fn clear_ocr_download_error(&self) {
        if let Ok(mut s) = self.ocr_download_status.lock()
            && matches!(*s, OcrDownloadStatus::Error(_))
        {
            *s = OcrDownloadStatus::Idle;
        }
    }

    /// Downloads the RapidOCR ONNX offline plugin package from the official release manifest,
    /// streaming progress updates back to the Settings UI and verifying SHA-256 before extraction.
    pub fn download_rapidocr_async(&self) {
        if let Ok(mut s) = self.ocr_download_status.lock() {
            if matches!(*s, OcrDownloadStatus::Downloading { .. }) {
                return;
            }
            *s = OcrDownloadStatus::Downloading {
                stage: "connecting".to_owned(),
                downloaded_bytes: 0,
                total_bytes: None,
            };
        }
        self.notify_ui(UiEvent::UpdateStateChanged);

        let manager = self.ocr_plugins.clone();
        let status_arc = Arc::clone(&self.ocr_download_status);
        let config_arc = Arc::clone(&self.config);
        let config_store = self.config_store.clone();
        let ui_tx = self.ui_tx.clone();

        thread::Builder::new()
            .name("clipboardx-rapidocr-download".to_owned())
            .spawn(move || {
                let result = download_rapidocr_manifest_and_install(&manager, &status_arc, &ui_tx);
                match result {
                    Ok(installed) => {
                        let plugin_id = installed.manifest.id;
                        if let Ok(mut cfg) = config_arc.lock() {
                            cfg.active_ocr_plugin = Some(plugin_id);
                            let _ = config_store.save(&cfg);
                        }
                        if let Ok(mut s) = status_arc.lock() {
                            *s = OcrDownloadStatus::Idle;
                        }
                        let _ = ui_tx.send(UiEvent::ConfigUpdated);
                    }
                    Err(err) => {
                        if let Ok(mut s) = status_arc.lock() {
                            *s = OcrDownloadStatus::Error(err);
                        }
                        let _ = ui_tx.send(UiEvent::UpdateStateChanged);
                    }
                }
            })
            .ok();
    }

    pub fn session_diagnostics(&self) -> DesktopSessionDiagnostics {
        collect_session_diagnostics()
    }

    pub fn can_self_update(&self) -> bool {
        std::env::current_exe()
            .map(|p| can_self_update_directory(&p))
            .unwrap_or(false)
    }

    pub fn update_status(&self) -> UpdateStatus {
        self.update_status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    pub fn check_updates_manual(&self) {
        if let Ok(mut s) = self.update_status.lock() {
            if matches!(*s, UpdateStatus::Checking | UpdateStatus::Downloading(_)) {
                return;
            }
            *s = UpdateStatus::Checking;
        }
        self.notify_ui(UiEvent::UpdateStateChanged);

        let channel = self.snapshot_config().update_channel;
        let status_arc = Arc::clone(&self.update_status);
        let ui_tx = self.ui_tx.clone();
        thread::Builder::new()
            .name("clipboardx-update-check".to_owned())
            .spawn(move || {
                let client = GitHubUpdateClient::new(env!("CARGO_PKG_VERSION"));
                let next = match client.check_latest(channel) {
                    Ok(Some(info)) => UpdateStatus::Available(info),
                    Ok(None) => UpdateStatus::UpToDate,
                    Err(err) => UpdateStatus::Error(err.to_string()),
                };
                if let Ok(mut s) = status_arc.lock() {
                    *s = next;
                }
                let _ = ui_tx.send(UiEvent::UpdateStateChanged);
            })
            .ok();
    }

    pub fn download_update_async(&self, info: UpdateReleaseInfo) {
        let ver = info.version.clone();
        if let Ok(mut s) = self.update_status.lock() {
            if matches!(*s, UpdateStatus::Downloading(_)) {
                return;
            }
            *s = UpdateStatus::Downloading(ver.clone());
        }
        self.notify_ui(UiEvent::UpdateStateChanged);

        let staging_dir = self.updates_dir.clone();
        let status_arc = Arc::clone(&self.update_status);
        let ui_tx = self.ui_tx.clone();
        thread::Builder::new()
            .name("clipboardx-update-download".to_owned())
            .spawn(move || {
                let client = GitHubUpdateClient::new(env!("CARGO_PKG_VERSION"));
                let exe_name = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.file_name().and_then(|s| s.to_str()).map(str::to_owned))
                    .unwrap_or_else(|| {
                        if cfg!(target_os = "windows") {
                            "clipboardx-app.exe".to_owned()
                        } else {
                            "clipboardx-app".to_owned()
                        }
                    });
                let next = match download_and_verify_release(
                    &info,
                    &staging_dir,
                    client.public_key_hex(),
                    current_platform_key(),
                    &exe_name,
                ) {
                    Ok(staged_binary) => UpdateStatus::ReadyToRestart {
                        version: ver,
                        staged_binary,
                    },
                    Err(err) => UpdateStatus::Error(err.to_string()),
                };
                if let Ok(mut s) = status_arc.lock() {
                    *s = next;
                }
                let _ = ui_tx.send(UiEvent::UpdateStateChanged);
            })
            .ok();
    }

    pub fn apply_staged_update(&self) -> Result<bool, ClipboardXError> {
        let staged = match self.update_status() {
            UpdateStatus::ReadyToRestart {
                version,
                staged_binary,
            } => (version, staged_binary),
            _ => return Ok(false),
        };

        let target = std::env::current_exe().map_err(|err| {
            ClipboardXError::Updater(clipboardx_model::UpdaterError::Install(err.to_string()))
        })?;
        match apply_atomic_binary_replacement(&staged.1, &target) {
            Ok(_) => {
                let relaunched = std::process::Command::new(&target).spawn().is_ok();
                if let Ok(mut s) = self.update_status.lock() {
                    *s = UpdateStatus::AppliedPendingRelaunch(staged.0);
                }
                self.notify_ui(UiEvent::UpdateStateChanged);
                Ok(relaunched)
            }
            Err(err) => {
                if let Ok(mut s) = self.update_status.lock() {
                    *s = UpdateStatus::Error(err.to_string());
                }
                self.notify_ui(UiEvent::UpdateStateChanged);
                Err(ClipboardXError::Updater(err))
            }
        }
    }
}

fn download_rapidocr_manifest_and_install(
    manager: &OcrPluginManager,
    status_arc: &Arc<Mutex<OcrDownloadStatus>>,
    ui_tx: &Sender<UiEvent>,
) -> Result<InstalledOcrPlugin, String> {
    let mut last_emit = std::time::Instant::now()
        .checked_sub(Duration::from_millis(200))
        .unwrap_or_else(std::time::Instant::now);
    let mut last_stage = "";

    manager
        .install_official_rapidocr_with_progress(|downloaded, total, stage| {
            let now = std::time::Instant::now();
            if stage != last_stage || now.duration_since(last_emit) >= Duration::from_millis(80) {
                last_stage = stage;
                last_emit = now;
                if let Ok(mut s) = status_arc.lock() {
                    *s = OcrDownloadStatus::Downloading {
                        stage: stage.to_owned(),
                        downloaded_bytes: downloaded,
                        total_bytes: total,
                    };
                }
                let _ = ui_tx.send(UiEvent::UpdateStateChanged);
            }
        })
        .map_err(|err| err.to_string())
}

fn spawn_startup_update_check(
    channel: clipboardx_model::UpdateChannel,
    status_arc: Arc<Mutex<UpdateStatus>>,
    ui_tx: Sender<UiEvent>,
) {
    thread::Builder::new()
        .name("clipboardx-startup-update".to_owned())
        .spawn(move || {
            thread::sleep(Duration::from_secs(4));
            let client = GitHubUpdateClient::new(env!("CARGO_PKG_VERSION"));
            match client.check_latest(channel) {
                Ok(Some(info)) => {
                    if let Ok(mut s) = status_arc.lock() {
                        *s = UpdateStatus::Available(info);
                    }
                    let _ = ui_tx.send(UiEvent::UpdateStateChanged);
                }
                Ok(None) => {}
                Err(err) => {
                    // Offline or rate-limited checks remain quiet during startup.
                    tracing::debug!(error = %err, "background update check skipped");
                }
            }
        })
        .ok();
}

fn spawn_background_ocr_worker(
    service: ClipboardService,
    config: Arc<Mutex<AppConfig>>,
    plugins_dir: PathBuf,
    ocr_rx: Receiver<ClipItemId>,
    ui_tx: Sender<UiEvent>,
) {
    thread::Builder::new()
        .name("clipboardx-ocr-worker".to_owned())
        .spawn(move || {
            while let Ok(item_id) = ocr_rx.recv() {
                let Ok(ClipboardPayload::Image { png_bytes, .. }) =
                    service.load_payload_for_paste(item_id, false)
                else {
                    continue;
                };

                let active_plugin = config
                    .lock()
                    .ok()
                    .and_then(|c| c.active_ocr_plugin.clone());
                let provider = resolve_active_provider(active_plugin.as_deref(), &plugins_dir);
                if !provider.is_available() {
                    continue;
                }

                match provider.recognize_png(&png_bytes, None) {
                    Ok(output) if !output.full_text.trim().is_empty() => {
                        if let Err(err) = service.update_ocr_text(item_id, &output.full_text) {
                            tracing::warn!(error = %err, "failed to store OCR text");
                        } else {
                            let _ = ui_tx.send(UiEvent::HistoryUpdated);
                        }
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::debug!(error = %err, provider = provider.id(), "OCR recognition skipped");
                    }
                }
            }
        })
        .ok();
}

fn ensure_runtime_dirs(paths: &AppPaths) -> Result<(), ClipboardXError> {
    for dir in [
        &paths.data_dir,
        &paths.blobs_dir,
        &paths.plugins_dir,
        &paths.updates_dir,
        &paths.logs_dir,
    ] {
        std::fs::create_dir_all(dir).map_err(|err| {
            ClipboardXError::Config(format!("create data directory failed: {err}"))
        })?;
    }
    Ok(())
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                dir_size(&path)
            } else {
                file_len(&path)
            }
        })
        .fold(0_u64, u64::saturating_add)
}
