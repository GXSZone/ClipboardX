#![forbid(unsafe_code)]

use std::time::Instant;

use clipboardx_model::{HotkeyEventKind, HotkeyTriggerMode, PlatformError};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

/// Manages registration and translation of OS global hotkeys.
pub struct HotkeyService {
    manager: GlobalHotKeyManager,
    toggle_id: u32,
    compact_id: u32,
    pause_id: u32,
    registered: Vec<HotKey>,
}

impl HotkeyService {
    pub fn register(
        toggle_spec: &str,
        compact_spec: &str,
        pause_spec: &str,
    ) -> Result<Self, PlatformError> {
        let manager = GlobalHotKeyManager::new()
            .map_err(|err| PlatformError::Hotkey(format!("init hotkey manager failed: {err}")))?;

        let toggle_hk = parse_hotkey(toggle_spec)?;
        let compact_hk = parse_hotkey(compact_spec)?;
        let pause_hk = parse_hotkey(pause_spec)?;

        manager.register(toggle_hk).map_err(|err| {
            PlatformError::Hotkey(format!("register {toggle_spec} failed: {err}"))
        })?;
        let _ = manager.register(compact_hk);
        let _ = manager.register(pause_hk);

        Ok(Self {
            manager,
            toggle_id: toggle_hk.id(),
            compact_id: compact_hk.id(),
            pause_id: pause_hk.id(),
            registered: vec![toggle_hk, compact_hk, pause_hk],
        })
    }

    /// Non-blocking poll for global hotkey events matching registered actions.
    pub fn poll_event(&self) -> Option<HotkeyEventKind> {
        while let Ok(ev) = GlobalHotKeyEvent::receiver().try_recv() {
            if ev.state != HotKeyState::Pressed {
                continue;
            }
            if ev.id == self.toggle_id {
                return Some(HotkeyEventKind::TogglePanelPressed);
            }
            if ev.id == self.compact_id {
                return Some(HotkeyEventKind::ToggleCompactPressed);
            }
            if ev.id == self.pause_id {
                return Some(HotkeyEventKind::TogglePausePressed);
            }
        }
        None
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        let _ = self.manager.unregister_all(&self.registered);
    }
}

/// Pure state machine driving `Toggle`, `HoldToPeek`, and `SmartHybrid` window visibility.
#[derive(Debug, Clone)]
pub struct HotkeyTriggerStateMachine {
    visible: bool,
    pressed_at: Option<Instant>,
    hold_session_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerOutcome {
    None,
    Show,
    Hide,
}

impl Default for HotkeyTriggerStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl HotkeyTriggerStateMachine {
    pub const fn new() -> Self {
        Self {
            visible: false,
            pressed_at: None,
            hold_session_active: false,
        }
    }

    pub const fn is_visible(&self) -> bool {
        self.visible
    }

    pub const fn needs_modifier_polling(&self) -> bool {
        self.visible && self.hold_session_active
    }

    /// Handles global hotkey press according to the configured trigger mode.
    pub fn on_hotkey_pressed(&mut self, mode: HotkeyTriggerMode, now: Instant) -> TriggerOutcome {
        if self.visible {
            self.visible = false;
            self.pressed_at = None;
            self.hold_session_active = false;
            return TriggerOutcome::Hide;
        }

        self.visible = true;
        self.pressed_at = Some(now);
        self.hold_session_active = matches!(
            mode,
            HotkeyTriggerMode::HoldToPeek | HotkeyTriggerMode::SmartHybrid
        );
        TriggerOutcome::Show
    }

    /// Polled periodically only while visible in Hold/SmartHybrid mode;
    /// returns `TriggerOutcome::Hide` when the user releases the modifier after holding.
    pub fn on_modifier_tick(
        &mut self,
        mode: HotkeyTriggerMode,
        modifier_still_held: bool,
        hold_threshold_ms: u64,
        now: Instant,
    ) -> TriggerOutcome {
        if !self.visible || !self.hold_session_active {
            return TriggerOutcome::None;
        }

        if modifier_still_held {
            return TriggerOutcome::None;
        }

        match mode {
            HotkeyTriggerMode::Toggle => {
                self.hold_session_active = false;
                TriggerOutcome::None
            }
            HotkeyTriggerMode::HoldToPeek => {
                self.visible = false;
                self.hold_session_active = false;
                self.pressed_at = None;
                TriggerOutcome::Hide
            }
            HotkeyTriggerMode::SmartHybrid => {
                let held_ms = self
                    .pressed_at
                    .map(|t| now.saturating_duration_since(t).as_millis() as u64)
                    .unwrap_or(0);
                self.hold_session_active = false;
                if held_ms >= hold_threshold_ms {
                    // Long hold: hide immediately upon release.
                    self.visible = false;
                    self.pressed_at = None;
                    TriggerOutcome::Hide
                } else {
                    // Quick tap (< 260ms): promote to persistent toggle mode.
                    TriggerOutcome::None
                }
            }
        }
    }

    /// Called when the window is hidden explicitly (Esc / paste / focus loss).
    pub fn mark_hidden(&mut self) {
        self.visible = false;
        self.pressed_at = None;
        self.hold_session_active = false;
    }
}

/// Parses human-readable shortcut strings such as `"Alt+V"`, `"Ctrl+\\"`, `"Cmd+Shift+V"`.
pub fn parse_hotkey(spec: &str) -> Result<HotKey, PlatformError> {
    let mut mods = Modifiers::empty();
    let mut key_code: Option<Code> = None;

    for raw in spec.split('+') {
        let token = raw.trim();
        let upper = token.to_ascii_uppercase();
        match upper.as_str() {
            "CTRL" | "CONTROL" => mods |= Modifiers::CONTROL,
            "ALT" | "OPTION" => mods |= Modifiers::ALT,
            "SHIFT" => mods |= Modifiers::SHIFT,
            "CMD" | "META" | "SUPER" | "WIN" => mods |= Modifiers::META,
            _ => {
                key_code = Some(parse_key_code(&upper).ok_or_else(|| {
                    PlatformError::Hotkey(format!("unsupported key '{token}' in '{spec}'"))
                })?);
            }
        }
    }

    let code =
        key_code.ok_or_else(|| PlatformError::Hotkey(format!("missing key code in '{spec}'")))?;
    let mods_opt = if mods.is_empty() { None } else { Some(mods) };
    Ok(HotKey::new(mods_opt, code))
}

fn parse_key_code(upper: &str) -> Option<Code> {
    match upper {
        "A" => Some(Code::KeyA),
        "B" => Some(Code::KeyB),
        "C" => Some(Code::KeyC),
        "D" => Some(Code::KeyD),
        "E" => Some(Code::KeyE),
        "F" => Some(Code::KeyF),
        "G" => Some(Code::KeyG),
        "H" => Some(Code::KeyH),
        "I" => Some(Code::KeyI),
        "J" => Some(Code::KeyJ),
        "K" => Some(Code::KeyK),
        "L" => Some(Code::KeyL),
        "M" => Some(Code::KeyM),
        "N" => Some(Code::KeyN),
        "O" => Some(Code::KeyO),
        "P" => Some(Code::KeyP),
        "Q" => Some(Code::KeyQ),
        "R" => Some(Code::KeyR),
        "S" => Some(Code::KeyS),
        "T" => Some(Code::KeyT),
        "U" => Some(Code::KeyU),
        "V" => Some(Code::KeyV),
        "W" => Some(Code::KeyW),
        "X" => Some(Code::KeyX),
        "Y" => Some(Code::KeyY),
        "Z" => Some(Code::KeyZ),
        "SPACE" => Some(Code::Space),
        "\\" | "BACKSLASH" => Some(Code::Backslash),
        "`" | "BACKQUOTE" => Some(Code::Backquote),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn parses_default_shortcuts() {
        assert!(parse_hotkey("Alt+V").is_ok());
        assert!(parse_hotkey("Ctrl+\\").is_ok());
        assert!(parse_hotkey("Ctrl+Alt+P").is_ok());
        assert!(parse_hotkey("Cmd+Shift+V").is_ok());
    }

    #[test]
    fn smart_hybrid_distinguishes_quick_tap_and_long_hold() {
        let mut sm = HotkeyTriggerStateMachine::new();
        let t0 = Instant::now();

        // Quick tap: press then release after 100ms -> stays visible.
        assert_eq!(
            sm.on_hotkey_pressed(HotkeyTriggerMode::SmartHybrid, t0),
            TriggerOutcome::Show
        );
        assert_eq!(
            sm.on_modifier_tick(
                HotkeyTriggerMode::SmartHybrid,
                false,
                260,
                t0 + Duration::from_millis(100)
            ),
            TriggerOutcome::None
        );
        assert!(sm.is_visible());

        // Second tap toggles off.
        assert_eq!(
            sm.on_hotkey_pressed(
                HotkeyTriggerMode::SmartHybrid,
                t0 + Duration::from_millis(300)
            ),
            TriggerOutcome::Hide
        );
        assert!(!sm.is_visible());

        // Long hold (>= 260ms): hides as soon as modifier is released.
        let t1 = t0 + Duration::from_secs(1);
        assert_eq!(
            sm.on_hotkey_pressed(HotkeyTriggerMode::SmartHybrid, t1),
            TriggerOutcome::Show
        );
        assert_eq!(
            sm.on_modifier_tick(
                HotkeyTriggerMode::SmartHybrid,
                true,
                260,
                t1 + Duration::from_millis(300)
            ),
            TriggerOutcome::None
        );
        assert_eq!(
            sm.on_modifier_tick(
                HotkeyTriggerMode::SmartHybrid,
                false,
                260,
                t1 + Duration::from_millis(400)
            ),
            TriggerOutcome::Hide
        );
        assert!(!sm.is_visible());
    }
}
