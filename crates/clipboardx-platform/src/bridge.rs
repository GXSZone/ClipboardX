#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};

use clipboardx_model::{
    ClipboardPayload, ForegroundWindowToken, PlatformBridge, PlatformError, SimulatedKey,
};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

use crate::clipboard::{clear_clipboard, read_text_snapshot, write_clipboard_payload};
use crate::ffi;
use crate::privacy::SelfWriteSuppressor;

/// Production desktop implementation of `clipboardx_model::PlatformBridge`.
#[derive(Clone)]
pub struct DesktopPlatformBridge {
    suppressor: SelfWriteSuppressor,
    enigo: Arc<Mutex<Option<Enigo>>>,
}

impl DesktopPlatformBridge {
    pub fn new(suppressor: SelfWriteSuppressor) -> Self {
        Self {
            suppressor,
            enigo: Arc::new(Mutex::new(None)),
        }
    }

    pub fn suppressor(&self) -> &SelfWriteSuppressor {
        &self.suppressor
    }

    fn with_enigo<F>(&self, f: F) -> Result<(), PlatformError>
    where
        F: FnOnce(&mut Enigo) -> Result<(), PlatformError>,
    {
        let mut guard = self
            .enigo
            .lock()
            .map_err(|_| PlatformError::InputSimulation("enigo mutex poisoned".to_owned()))?;
        if guard.is_none() {
            let instance = Enigo::new(&Settings::default()).map_err(|err| {
                PlatformError::InputSimulation(format!("init enigo failed: {err}"))
            })?;
            *guard = Some(instance);
        }
        let Some(enigo) = guard.as_mut() else {
            return Err(PlatformError::InputSimulation(
                "enigo unavailable".to_owned(),
            ));
        };
        f(enigo)
    }
}

impl PlatformBridge for DesktopPlatformBridge {
    fn read_text_snapshot(&self) -> Result<Option<String>, PlatformError> {
        read_text_snapshot()
    }

    fn write_clipboard(&self, payload: &ClipboardPayload) -> Result<(), PlatformError> {
        write_clipboard_payload(payload, &self.suppressor)
    }

    fn clear_clipboard(&self) -> Result<(), PlatformError> {
        clear_clipboard()
    }

    fn capture_foreground_window(&self) -> Option<ForegroundWindowToken> {
        ffi::capture_foreground_window()
    }

    fn restore_foreground_window(&self, token: ForegroundWindowToken) -> Result<(), PlatformError> {
        ffi::restore_foreground_window(token)
    }

    fn simulate_paste(&self) -> Result<(), PlatformError> {
        if self.is_restricted_wayland() {
            return Err(PlatformError::Unsupported(
                "Wayland session blocks synthetic paste".to_owned(),
            ));
        }

        self.with_enigo(|enigo| {
            #[cfg(target_os = "macos")]
            let modifier = Key::Meta;
            #[cfg(not(target_os = "macos"))]
            let modifier = Key::Control;

            enigo
                .key(modifier, Direction::Press)
                .map_err(|e| PlatformError::InputSimulation(e.to_string()))?;
            enigo
                .key(Key::Unicode('v'), Direction::Click)
                .map_err(|e| PlatformError::InputSimulation(e.to_string()))?;
            enigo
                .key(modifier, Direction::Release)
                .map_err(|e| PlatformError::InputSimulation(e.to_string()))?;
            Ok(())
        })
    }

    fn simulate_key_tap(&self, key: SimulatedKey) -> Result<(), PlatformError> {
        if self.is_restricted_wayland() {
            return Err(PlatformError::Unsupported(
                "Wayland session blocks synthetic key taps".to_owned(),
            ));
        }

        let target = match key {
            SimulatedKey::Tab => Key::Tab,
            SimulatedKey::Enter => Key::Return,
        };

        self.with_enigo(|enigo| {
            enigo
                .key(target, Direction::Click)
                .map_err(|e| PlatformError::InputSimulation(e.to_string()))
        })
    }

    fn is_hotkey_modifier_held(&self, hotkey_spec: &str) -> bool {
        ffi::is_hotkey_modifier_held(hotkey_spec)
    }

    fn is_restricted_wayland(&self) -> bool {
        ffi::is_restricted_wayland()
    }
}
