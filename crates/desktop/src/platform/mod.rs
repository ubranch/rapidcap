//! Everything the app needs from the OS that GPUI does not surface.
//!
//! The hotkeys, the tray and the event pump below are the same code on both
//! platforms - `global-hotkey` and `tray-icon` already carry their own backends,
//! so there is nothing here to split. Window manipulation is the opposite: it is
//! Win32 on Windows and AppKit on macOS with no shared shape at all, so it lives
//! in a sibling module per platform, each exporting the same names.

use std::{
    cell::{Cell, RefCell},
    path::Path,
};

use anyhow::Context as _;
use async_channel::Receiver;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use gpui::Global;
use rapidcap_capture::{CaptureCommand, CaptureKind, CaptureState};
use serde_json::{Value, json};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, accelerator::Accelerator},
};

use crate::tray::{self, TrayState};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::{
    SingleInstance, drag_main_window, exclude_from_capture, hide_main_window, hide_recording_frame,
    lock_window_size, monitor_under_cursor, open_path, place_main_window, place_window,
    remember_main_window, show_main_window, show_recording_frame, text_scale, window_drag_grab,
    window_target_at,
};
#[cfg(windows)]
pub use windows::{
    SingleInstance, drag_main_window, exclude_from_capture, hide_main_window, hide_recording_frame,
    lock_window_size, monitor_under_cursor, open_path, place_main_window, place_window,
    remember_main_window, show_main_window, show_recording_frame, text_scale, window_drag_grab,
    window_target_at,
};

const APP_ID: &str = "com.inspire.rapidcap";
const MENU_SHOW: &str = "show";
const MENU_OUTPUT: &str = "output";
const MENU_EXIT: &str = "exit";
const MENU_STATE: &str = "state";

/// The commands a tray item can dispatch, and the id its item carries.
///
/// A table rather than a `match`, because the lookup runs both ways: the menu
/// builder needs an id for a command, and `MenuEvent` - a `'static` closure
/// that never sees the app - needs the command back out of an id.
const MENU_COMMANDS: [(&str, CaptureCommand); 5] = [
    ("capture-region", CaptureCommand::CaptureRegion),
    ("capture-window", CaptureCommand::CaptureActiveWindow),
    ("record-video", CaptureCommand::ToggleVideo),
    ("record-gif", CaptureCommand::ToggleGif),
    ("toggle-pause", CaptureCommand::TogglePause),
];

fn menu_id(command: CaptureCommand) -> &'static str {
    MENU_COMMANDS
        .iter()
        .find(|(_, candidate)| *candidate == command)
        .map(|(id, _)| *id)
        .expect("every menu command has an id")
}

fn menu_command(id: &str) -> Option<CaptureCommand> {
    MENU_COMMANDS
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .map(|(_, command)| *command)
}

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
/// will lose the race and fail to register. Startup carries on without it - the
/// panel's buttons never needed the chord - and the command lands in
/// [`PlatformRuntime::unavailable_hotkeys`], which is what stops the panel from
/// printing a key that will never fire.
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

/// One command the tray menu offers, and the words it offers it with.
///
/// The label is not the command's name. `ToggleVideo` reads "Record video" in
/// an idle menu and "Stop recording" in a recording one, and a menu that says
/// "Record video" over a running recording is describing the wrong half of a
/// toggle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MenuAction {
    command: CaptureCommand,
    label: &'static str,
}

impl MenuAction {
    const fn new(command: CaptureCommand, label: &'static str) -> Self {
        Self { command, label }
    }

    /// Stopping a recording means toggling the kind that is running. Sending
    /// `ToggleVideo` at a GIF is answered with `CommandError::Busy`, so the item
    /// has to carry the live kind rather than a fixed command.
    const fn stop(kind: CaptureKind) -> Self {
        let command = match kind {
            CaptureKind::Gif => CaptureCommand::ToggleGif,
            _ => CaptureCommand::ToggleVideo,
        };
        Self::new(command, "Stop recording")
    }
}

/// The capture commands the menu should offer in a state.
///
/// Only the ones `AppController::dispatch` would accept. An item that answers
/// `CommandError::Busy` is worse than no item at all: the user clicked
/// something RapidCap drew for them and nothing happened.
fn menu_actions(state: &CaptureState) -> Vec<MenuAction> {
    match state {
        // `dispatch` clears the error state before it reads the command, so a
        // failed capture offers everything an idle one does.
        CaptureState::Idle | CaptureState::Error(_) => vec![
            MenuAction::new(CaptureCommand::CaptureRegion, "Capture region"),
            MenuAction::new(CaptureCommand::CaptureActiveWindow, "Capture window"),
            MenuAction::new(CaptureCommand::ToggleVideo, "Record video"),
        ],
        CaptureState::Recording(kind) => vec![
            MenuAction::new(CaptureCommand::TogglePause, "Pause"),
            MenuAction::stop(*kind),
        ],
        CaptureState::Paused(kind) => vec![
            MenuAction::new(CaptureCommand::TogglePause, "Resume"),
            MenuAction::stop(*kind),
        ],
        // Selecting draws a full-screen overlay with the tray behind it,
        // Countdown lasts three seconds, and Finalizing accepts nothing at all.
        // None of the three has a command worth offering.
        CaptureState::Selecting(_)
        | CaptureState::Countdown(_, _)
        | CaptureState::Finalizing(_) => Vec::new(),
    }
}

/// The chord a menu item prints beside its label.
///
/// Drawn text, not a binding. The hotkeys are global and fire whether the menu
/// is open or not, and nothing here runs `TranslateAccelerator` over the table
/// `muda` builds alongside the item.
fn menu_accelerator(command: CaptureCommand) -> Option<Accelerator> {
    hotkey_specs()
        .into_iter()
        .find(|spec| spec.command == command)
        .map(|spec| Accelerator::new(Some(spec.modifiers), spec.code))
}

/// The whole tray menu for a capture state.
///
/// Rebuilt rather than mutated: `muda` has no call that swaps a run of items,
/// and the state changes a handful of times per capture, not per frame.
fn tray_menu(state: &CaptureState) -> tray_icon::menu::Result<Menu> {
    // Disabled because it is a label, not a command. Greying it is what tells a
    // header apart from an item in a menu the OS draws.
    let header = MenuItem::with_id(
        MENU_STATE,
        TrayState::from_capture(state).tooltip(state),
        false,
        None,
    );
    let actions: Vec<MenuItem> = menu_actions(state)
        .into_iter()
        .map(|action| {
            MenuItem::with_id(
                menu_id(action.command),
                action.label,
                true,
                menu_accelerator(action.command),
            )
        })
        .collect();
    let show = MenuItem::with_id(MENU_SHOW, "Show RapidCap", true, None);
    let output = MenuItem::with_id(MENU_OUTPUT, "Open output folder", true, None);
    let exit = MenuItem::with_id(MENU_EXIT, "Exit", true, None);
    // One separator per gap. A single item cannot sit in a menu twice.
    let separators = [
        PredefinedMenuItem::separator(),
        PredefinedMenuItem::separator(),
        PredefinedMenuItem::separator(),
    ];

    let mut items: Vec<&dyn IsMenuItem> = vec![&header, &separators[0]];
    items.extend(actions.iter().map(|item| item as &dyn IsMenuItem));
    if !actions.is_empty() {
        items.push(&separators[1]);
    }
    items.extend([&show as &dyn IsMenuItem, &output, &separators[2], &exit]);
    Menu::with_items(&items)
}

pub struct PlatformRuntime {
    receiver: Receiver<PlatformEvent>,
    _hotkeys: GlobalHotKeyManager,
    /// The commands whose chord `RegisterHotKey` refused, because some other
    /// running app claimed it first. Startup carries on without them - the
    /// panel's buttons do not need the chord - but the panel has to stop
    /// printing a key that will never fire.
    unavailable: Vec<CaptureCommand>,
    tray: TrayIcon,
    tray_state: Cell<Option<TrayState>>,
    /// The state the menu on screen was built for. Its shape and its header
    /// both follow the whole `CaptureState`, not the five-way `TrayState` the
    /// icon uses, so the icon's guard cannot stand in for this one.
    menu_state: RefCell<CaptureState>,
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
        let mut unavailable = Vec::new();
        for (spec, hotkey) in specs.iter().zip(&hotkeys) {
            if let Err(error) = manager.register(*hotkey) {
                tracing::warn!(?hotkey, %error, "global hotkey unavailable");
                unavailable.push(spec.command);
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

        let menu = tray_menu(&CaptureState::Idle)?;
        let menu_sender = sender.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let id = event.id().0.as_str();
            let event = match id {
                MENU_SHOW => Some(PlatformEvent::Show),
                MENU_OUTPUT => Some(PlatformEvent::OpenOutput),
                MENU_EXIT => Some(PlatformEvent::Exit),
                _ => menu_command(id).map(PlatformEvent::Capture),
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
            unavailable,
            tray,
            tray_state: Cell::new(Some(TrayState::Idle)),
            menu_state: RefCell::new(CaptureState::Idle),
        })
    }

    pub fn receiver(&self) -> Receiver<PlatformEvent> {
        self.receiver.clone()
    }

    /// The commands whose chord another app already owned at startup.
    pub fn unavailable_hotkeys(&self) -> &[CaptureCommand] {
        &self.unavailable
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
        if *self.menu_state.borrow() != *state {
            match tray_menu(state) {
                Ok(menu) => {
                    self.tray.set_menu(Some(Box::new(menu)));
                    self.menu_state.replace(state.clone());
                }
                Err(error) => tracing::warn!(%error, "rebuild tray menu"),
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
    fn stopping_from_the_tray_toggles_the_kind_that_is_running() {
        // A fixed "Stop recording" command would leave a GIF unstoppable from
        // the tray: `ToggleVideo` at a running GIF is answered `Busy`.
        assert!(
            CaptureState::Recording(CaptureKind::Gif)
                .stop(CaptureKind::Video)
                .is_err()
        );

        let gif = menu_actions(&CaptureState::Recording(CaptureKind::Gif));
        let video = menu_actions(&CaptureState::Paused(CaptureKind::Video));
        assert_eq!(gif.last().unwrap().command, CaptureCommand::ToggleGif);
        assert_eq!(video.last().unwrap().command, CaptureCommand::ToggleVideo);
        assert_eq!(gif.last().unwrap().label, "Stop recording");
    }

    #[test]
    fn the_menu_offers_nothing_a_state_would_reject() {
        for state in [
            CaptureState::Selecting(CaptureKind::Video),
            CaptureState::Countdown(CaptureKind::Gif, 3),
            CaptureState::Finalizing(CaptureKind::Video),
        ] {
            assert!(
                menu_actions(&state).is_empty(),
                "{state:?} offered a command"
            );
        }

        let commands = |state| -> Vec<CaptureCommand> {
            menu_actions(&state)
                .into_iter()
                .map(|action| action.command)
                .collect()
        };
        // Nothing to pause when nothing is running, and no new capture to start
        // over one that is.
        assert!(!commands(CaptureState::Idle).contains(&CaptureCommand::TogglePause));
        let recording = commands(CaptureState::Recording(CaptureKind::Video));
        assert!(!recording.contains(&CaptureCommand::CaptureRegion));
        assert!(!recording.contains(&CaptureCommand::CaptureActiveWindow));
        // A failed capture is cleared by `dispatch`, so it offers what idle does.
        assert_eq!(
            commands(CaptureState::Error("disk full".to_string())),
            commands(CaptureState::Idle)
        );
    }

    #[test]
    fn every_menu_command_id_round_trips() {
        // The event handler only ever sees the id string. An id that does not
        // map back is a menu item that silently does nothing when clicked.
        for (id, command) in MENU_COMMANDS {
            assert_eq!(menu_id(command), id);
            assert_eq!(menu_command(id), Some(command));
        }
        assert_eq!(menu_command(MENU_EXIT), None);
        assert_eq!(menu_command(MENU_STATE), None);
    }

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
