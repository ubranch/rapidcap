//! Everything the app needs from the OS that GPUI does not surface.
//!
//! The hotkeys, the tray and the event pump below are the same code on both
//! platforms - `global-hotkey` and `tray-icon` already carry their own backends,
//! so there is nothing here to split. Window manipulation is the opposite: it is
//! Win32 on Windows and AppKit on macOS with no shared shape at all, so it lives
//! in a sibling module per platform, each exporting the same names.

use std::{cell::Cell, path::Path};

use anyhow::Context as _;
use async_channel::Receiver;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use gpui::Global;
use rapidcap_capture::{CaptureCommand, CaptureState};
use serde_json::{Value, json};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

use crate::tray::{self, TrayState};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::{
    SingleInstance, drag_main_window, exclude_from_capture, hide_main_window, hide_recording_frame,
    lock_window_size, monitor_under_cursor, open_folder, place_main_window, place_window,
    remember_main_window, show_main_window, show_recording_frame, text_scale, window_drag_grab,
    window_target_at,
};
#[cfg(windows)]
pub use windows::{
    SingleInstance, drag_main_window, exclude_from_capture, hide_main_window, hide_recording_frame,
    lock_window_size, monitor_under_cursor, open_folder, place_main_window, place_window,
    remember_main_window, show_main_window, show_recording_frame, text_scale, window_drag_grab,
    window_target_at,
};

const APP_ID: &str = "com.inspire.rapidcap";
const MENU_SHOW: &str = "show";
const MENU_OUTPUT: &str = "output";
const MENU_EXIT: &str = "exit";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotkeySpec {
    modifiers: Modifiers,
    code: Code,
    command: CaptureCommand,
}

impl HotkeySpec {
    const fn new(modifiers: Modifiers, code: Code, command: CaptureCommand) -> Self {
        Self {
            modifiers,
            code,
            command,
        }
    }

    fn hotkey(self) -> HotKey {
        HotKey::new(Some(self.modifiers), self.code)
    }

    /// How the shortcut reads to a user: `Alt+E`, `Ctrl+Shift+G`.
    ///
    /// Derived from the spec rather than written out again, so the pill on a
    /// card cannot promise a key the app never registered.
    /// The shortcut as the platform writes it: `Ctrl+Shift+W` on Windows and
    /// `⇧⌘W` on macOS, where symbols in a fixed order and no separator are the
    /// convention and spelled-out modifier names look out of place.
    pub fn label(self) -> String {
        let mac = cfg!(target_os = "macos");
        let debug = format!("{:?}", self.code);
        let key = debug.strip_prefix("Key").unwrap_or(&debug);
        let mut parts = Vec::with_capacity(4);
        // Control, option, shift, command - the Mac order, which is fixed.
        if self.modifiers.contains(Modifiers::CONTROL) {
            parts.push(if mac { "⌃" } else { "Ctrl" });
        }
        if mac && self.modifiers.contains(Modifiers::ALT) {
            parts.push("⌥");
        }
        if self.modifiers.contains(Modifiers::SHIFT) {
            parts.push(if mac { "⇧" } else { "Shift" });
        }
        if self.modifiers.contains(Modifiers::SUPER) {
            parts.push(if mac { "⌘" } else { "Win" });
        }
        if !mac && self.modifiers.contains(Modifiers::ALT) {
            parts.push("Alt");
        }
        parts.push(key);
        parts.join(if mac { "" } else { "+" })
    }
}

/// The shortcut a control should print, or `None` for a command nobody bound.
pub fn shortcut_label(command: CaptureCommand) -> Option<String> {
    hotkey_specs()
        .into_iter()
        .find(|spec| spec.command == command)
        .map(HotkeySpec::label)
}

/// The five shortcuts, one per command.
///
/// Alt+Q and Alt+E are the two the user asked for by name, so they are the two
/// that ship, even though ShareX uses them too. `RegisterHotKey` is
/// first-come-first-served, so on a machine already running ShareX one of them
/// will lose the race and fail to register - `PlatformRuntime` logs that rather
/// than failing to start, and the panel's buttons still work.
///
/// The remaining three have no requested binding, so they stay on
/// primary+Shift+letter, mnemonic per command and unclaimed by either OS.
///
/// "Primary" is Ctrl on Windows and Command on macOS. Ctrl+Shift+letter is not
/// a Mac chord - the keys a Mac user reaches for with those fingers are
/// Cmd+Shift - so that modifier follows the platform. Alt+Q and Alt+E stay
/// Option+Q and Option+E, because those two were asked for by key, not by role.
pub fn hotkey_specs() -> [HotkeySpec; 5] {
    #[cfg(target_os = "macos")]
    const PRIMARY: Modifiers = Modifiers::SUPER;
    #[cfg(not(target_os = "macos"))]
    const PRIMARY: Modifiers = Modifiers::CONTROL;
    const CTRL_SHIFT: Modifiers = PRIMARY.union(Modifiers::SHIFT);
    [
        HotkeySpec::new(Modifiers::ALT, Code::KeyE, CaptureCommand::CaptureRegion),
        HotkeySpec::new(CTRL_SHIFT, Code::KeyW, CaptureCommand::CaptureActiveWindow),
        HotkeySpec::new(Modifiers::ALT, Code::KeyQ, CaptureCommand::ToggleVideo),
        HotkeySpec::new(CTRL_SHIFT, Code::KeyG, CaptureCommand::ToggleGif),
        HotkeySpec::new(CTRL_SHIFT, Code::KeyP, CaptureCommand::TogglePause),
    ]
}

pub fn probe_payload(output: impl AsRef<Path>) -> Value {
    json!({
        "app_id": APP_ID,
        "version": env!("CARGO_PKG_VERSION"),
        "output": output.as_ref(),
        "hotkeys": hotkey_specs().map(HotkeySpec::label)
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformEvent {
    Capture(CaptureCommand),
    Show,
    OpenOutput,
    Exit,
}

pub struct PlatformRuntime {
    receiver: Receiver<PlatformEvent>,
    _hotkeys: GlobalHotKeyManager,
    tray: TrayIcon,
    tray_state: Cell<Option<TrayState>>,
}

impl Global for PlatformRuntime {}

/// `tray_icon` wants an owned buffer; the rasteriser hands one over.
fn tray_pixels(state: TrayState) -> Vec<u8> {
    tray::rgba(state)
}

impl PlatformRuntime {
    pub fn start() -> anyhow::Result<Self> {
        let (sender, receiver) = async_channel::unbounded();
        let specs = hotkey_specs();
        let hotkeys: Vec<_> = specs.iter().copied().map(HotkeySpec::hotkey).collect();
        let manager = GlobalHotKeyManager::new().context("create global hotkey manager")?;
        for hotkey in &hotkeys {
            if let Err(error) = manager.register(*hotkey) {
                tracing::warn!(?hotkey, %error, "global hotkey unavailable");
            }
        }

        let hotkey_sender = sender.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() != HotKeyState::Pressed {
                return;
            }
            if let Some(spec) = specs.iter().find(|spec| spec.hotkey().id() == event.id()) {
                let _ = hotkey_sender.try_send(PlatformEvent::Capture(spec.command));
            }
        }));

        let menu = Menu::with_items(&[
            &MenuItem::with_id(MENU_SHOW, "Show RapidCap", true, None),
            &MenuItem::with_id(MENU_OUTPUT, "Open output folder", true, None),
            &MenuItem::with_id(MENU_EXIT, "Exit", true, None),
        ])?;
        let menu_sender = sender.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let event = match event.id().0.as_str() {
                MENU_SHOW => Some(PlatformEvent::Show),
                MENU_OUTPUT => Some(PlatformEvent::OpenOutput),
                MENU_EXIT => Some(PlatformEvent::Exit),
                _ => None,
            };
            if let Some(event) = event {
                let _ = menu_sender.try_send(event);
            }
        }));

        let tray_sender = sender;
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                let _ = tray_sender.try_send(PlatformEvent::Show);
            }
        }));

        let tray = TrayIconBuilder::new()
            .with_tooltip("RapidCap")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_icon(Icon::from_rgba(
                tray_pixels(TrayState::Idle),
                tray::SIZE,
                tray::SIZE,
            )?)
            .build()?;

        Ok(Self {
            receiver,
            _hotkeys: manager,
            tray,
            tray_state: Cell::new(Some(TrayState::Idle)),
        })
    }

    pub fn receiver(&self) -> Receiver<PlatformEvent> {
        self.receiver.clone()
    }

    /// Push the current capture state onto the tray icon and its tooltip.
    ///
    /// Repainting the icon is a Win32 round trip, so the last painted state is
    /// remembered and an unchanged state is a no-op — this is called from a
    /// controller observer that also fires on target and settings changes.
    pub fn show_capture_state(&self, state: &CaptureState) {
        let next = TrayState::from_capture(state);
        if self.tray_state.get() != Some(next) {
            match Icon::from_rgba(tray_pixels(next), tray::SIZE, tray::SIZE) {
                Ok(icon) => {
                    if let Err(error) = self.tray.set_icon(Some(icon)) {
                        tracing::warn!(%error, "update tray icon");
                    }
                    self.tray_state.set(Some(next));
                }
                Err(error) => tracing::warn!(%error, "build tray icon"),
            }
        }
        if let Err(error) = self.tray.set_tooltip(Some(next.tooltip(state))) {
            tracing::warn!(%error, "update tray tooltip");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_except_cancel_has_exactly_one_shortcut() {
        let specs = hotkey_specs();
        let mut commands: Vec<_> = specs.iter().map(|spec| spec.command).collect();
        commands.sort_by_key(|command| format!("{command:?}"));
        commands.dedup();
        assert_eq!(
            commands.len(),
            specs.len(),
            "two shortcuts fire the same command"
        );
    }

    #[test]
    fn the_two_requested_shortcuts_are_bound() {
        // Alt+Q and Alt+E were asked for by name. They collide with ShareX,
        // which is accepted; what is not accepted is quietly drifting back to
        // some other binding the next time this table is edited.
        let specs = hotkey_specs();
        let bound = |modifiers, code| {
            specs
                .iter()
                .find(|spec| spec.modifiers == modifiers && spec.code == code)
                .map(|spec| spec.command)
        };
        assert_eq!(
            bound(Modifiers::ALT, Code::KeyQ),
            Some(CaptureCommand::ToggleVideo),
            "Alt+Q must record"
        );
        assert_eq!(
            bound(Modifiers::ALT, Code::KeyE),
            Some(CaptureCommand::CaptureRegion),
            "Alt+E must capture"
        );
    }

    #[test]
    fn probe_payload_is_machine_readable() {
        let value = probe_payload("/captures");
        assert_eq!(value["app_id"], APP_ID);
        assert_eq!(value["output"], "/captures");
        assert_eq!(value["hotkeys"].as_array().unwrap().len(), 5);
    }
}
