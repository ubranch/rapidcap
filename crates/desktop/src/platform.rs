use std::{os::windows::ffi::OsStrExt as _, path::Path, thread, time::Duration};

use anyhow::Context as _;
use async_channel::Receiver;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use gpui::Global;
use rapidcap_capture::CaptureCommand;
use serde_json::{Value, json};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
        System::Threading::CreateMutexW,
        UI::{
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                FindWindowW, SW_RESTORE, SW_SHOWNORMAL, SetForegroundWindow, ShowWindow,
            },
        },
    },
    core::{PCWSTR, w},
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
}

pub fn hotkey_specs() -> [HotkeySpec; 5] {
    [
        HotkeySpec::new(Modifiers::ALT, Code::KeyQ, CaptureCommand::CaptureRegion),
        HotkeySpec::new(
            Modifiers::ALT,
            Code::PrintScreen,
            CaptureCommand::CaptureActiveWindow,
        ),
        HotkeySpec::new(Modifiers::ALT, Code::KeyE, CaptureCommand::ToggleVideo),
        HotkeySpec::new(
            Modifiers::SHIFT | Modifiers::ALT,
            Code::PrintScreen,
            CaptureCommand::ToggleVideo,
        ),
        HotkeySpec::new(
            Modifiers::CONTROL | Modifiers::SHIFT,
            Code::PrintScreen,
            CaptureCommand::ToggleGif,
        ),
    ]
}

pub fn probe_payload(output: impl AsRef<Path>) -> Value {
    json!({
        "app_id": APP_ID,
        "version": env!("CARGO_PKG_VERSION"),
        "output": output.as_ref(),
        "hotkeys": ["Alt+Q", "Alt+PrintScreen", "Alt+E", "Shift+Alt+PrintScreen", "Ctrl+Shift+PrintScreen"]
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
    _tray: TrayIcon,
}

impl Global for PlatformRuntime {}

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
            if let Some(spec) = specs
                .iter()
                .find(|spec| spec.hotkey().id() == event.id())
            {
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
            .with_icon(Icon::from_rgba([34, 92, 197, 255].repeat(16 * 16), 16, 16)?)
            .build()?;

        Ok(Self {
            receiver,
            _hotkeys: manager,
            _tray: tray,
        })
    }

    pub fn receiver(&self) -> Receiver<PlatformEvent> {
        self.receiver.clone()
    }
}

pub struct SingleInstance(HANDLE);

impl SingleInstance {
    pub fn acquire() -> anyhow::Result<Option<Self>> {
        let handle = unsafe { CreateMutexW(None, false, w!("Local\\com.inspire.rapidcap")) }
            .context("create RapidCap instance mutex")?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) }.context("close secondary instance mutex")?;
            activate_existing_window();
            return Ok(None);
        }
        Ok(Some(Self(handle)))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn activate_existing_window() {
    // ponytail: title lookup is enough for one-window app; add IPC only if routing grows.
    for _ in 0..20 {
        if let Ok(window) = unsafe { FindWindowW(PCWSTR::null(), w!("RapidCap")) } {
            unsafe {
                let _ = ShowWindow(window, SW_RESTORE);
                let _ = SetForegroundWindow(window);
            }
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

pub fn open_folder(path: &Path) -> anyhow::Result<()> {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        anyhow::bail!("ShellExecuteW failed with code {}", result.0 as isize);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharex_hotkeys_map_to_rapidcap_commands() {
        assert_eq!(
            hotkey_specs(),
            [
                HotkeySpec::new(Modifiers::ALT, Code::KeyQ, CaptureCommand::CaptureRegion),
                HotkeySpec::new(
                    Modifiers::ALT,
                    Code::PrintScreen,
                    CaptureCommand::CaptureActiveWindow,
                ),
                HotkeySpec::new(Modifiers::ALT, Code::KeyE, CaptureCommand::ToggleVideo),
                HotkeySpec::new(
                    Modifiers::SHIFT | Modifiers::ALT,
                    Code::PrintScreen,
                    CaptureCommand::ToggleVideo,
                ),
                HotkeySpec::new(
                    Modifiers::CONTROL | Modifiers::SHIFT,
                    Code::PrintScreen,
                    CaptureCommand::ToggleGif,
                ),
            ]
        );
    }

    #[test]
    fn probe_payload_is_machine_readable() {
        let value = probe_payload("C:/Captures");
        assert_eq!(value["app_id"], APP_ID);
        assert_eq!(value["output"], "C:/Captures");
        assert_eq!(value["hotkeys"].as_array().unwrap().len(), 5);
    }
}
