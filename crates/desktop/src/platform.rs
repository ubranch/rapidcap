use std::{
    cell::Cell,
    mem::size_of,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    os::windows::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use anyhow::Context as _;
use async_channel::Receiver;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use gpui::Global;
use rapidcap_capture::{CaptureCommand, CaptureState, CaptureTarget, PhysicalRegion};
use serde_json::{Value, json};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, COLORREF, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HINSTANCE, HWND,
            LPARAM, LRESULT, POINT, RECT, WPARAM,
        },
        Graphics::{
            Dwm::{
                DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_EXTENDED_FRAME_BOUNDS,
                DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWM_WINDOW_CORNER_PREFERENCE,
                DwmGetWindowAttribute, DwmSetWindowAttribute,
            },
            Gdi::{
                CombineRgn, CreateRectRgn, CreateSolidBrush, DeleteObject, GetMonitorInfoW,
                MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, RGN_DIFF,
                SetWindowRgn,
            },
        },
        System::LibraryLoader::GetModuleHandleW,
        System::Threading::{
            CreateMutexW, GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        },
        UI::{
            HiDpi::GetDpiForWindow,
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, FindWindowW, GWL_STYLE, GW_HWNDNEXT,
                GetClientRect, GetCursorPos, GetTopWindow, GetWindow, GetWindowLongPtrW,
                GetWindowRect, GetWindowThreadProcessId, HWND_NOTOPMOST, HWND_TOPMOST, IsIconic,
                IsWindowVisible, LWA_ALPHA, RegisterClassW, SW_HIDE, SW_RESTORE, SW_SHOW,
                SW_SHOWNORMAL, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                SWP_NOZORDER, SWP_SHOWWINDOW, SetForegroundWindow, SetLayeredWindowAttributes,
                SetWindowLongPtrW, SetWindowPos, ShowWindow, WNDCLASSW,
                WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
                WS_CAPTION, WS_EX_TRANSPARENT, WS_MAXIMIZEBOX, WS_POPUP, WS_SYSMENU,
                WS_THICKFRAME,
            },
        },
    },
    core::{PCWSTR, PWSTR, w},
};

use crate::tray::{self, TrayState};

const APP_ID: &str = "com.inspire.rapidcap";
const MENU_SHOW: &str = "show";
const MENU_OUTPUT: &str = "output";
const MENU_EXIT: &str = "exit";

pub fn window_target_at(point: (i32, i32)) -> anyhow::Result<CaptureTarget> {
    let current_process = unsafe { GetCurrentProcessId() };
    let mut hwnd = unsafe { GetTopWindow(None) }.context("find top window")?;
    loop {
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process_id)) };
        if unsafe { IsWindowVisible(hwnd).as_bool() }
            && !unsafe { IsIconic(hwnd).as_bool() }
            && let Ok(rect) = window_rect(hwnd)
            && window_candidate_contains(rect, point, process_id, current_process)
            && let Ok(target) = capture_target_for_hwnd(hwnd)
        {
            return Ok(target);
        }
        hwnd = unsafe { GetWindow(hwnd, GW_HWNDNEXT) }.context("walk top-level windows")?;
    }
}

fn window_candidate_contains(
    rect: RECT,
    point: (i32, i32),
    process_id: u32,
    current_process: u32,
) -> bool {
    process_id != current_process
        && point.0 >= rect.left
        && point.0 < rect.right
        && point.1 >= rect.top
        && point.1 < rect.bottom
}

fn window_rect(hwnd: HWND) -> anyhow::Result<RECT> {
    let mut rect = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut rect).cast(),
            size_of::<RECT>() as u32,
        )
    }
    .context("read foreground window bounds")?;
    Ok(rect)
}

fn capture_target_for_hwnd(hwnd: HWND) -> anyhow::Result<CaptureTarget> {
    let rect = window_rect(hwnd)?;
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process_id)) };
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
        .context("open foreground process")?;
    let process_name = process_name(process);
    unsafe { CloseHandle(process) }.context("close foreground process")?;

    Ok(CaptureTarget::Window {
        hwnd: hwnd.0 as isize,
        region: PhysicalRegion {
            x: rect.left,
            y: rect.top,
            width: (rect.right - rect.left) as u32,
            height: (rect.bottom - rect.top) as u32,
        },
        process_name: process_name?,
    })
}

fn process_name(process: HANDLE) -> anyhow::Result<String> {
    let mut buffer = vec![0_u16; 32_768];
    let mut length = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &raw mut length,
        )
    }
    .context("read foreground process name")?;
    Ok(
        PathBuf::from(String::from_utf16(&buffer[..length as usize])?)
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("foreground process has no file name"))?
            .to_owned(),
    )
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
    pub fn label(self) -> String {
        let debug = format!("{:?}", self.code);
        let key = debug.strip_prefix("Key").unwrap_or(&debug);
        let mut parts = Vec::with_capacity(4);
        if self.modifiers.contains(Modifiers::CONTROL) {
            parts.push("Ctrl");
        }
        if self.modifiers.contains(Modifiers::SHIFT) {
            parts.push("Shift");
        }
        if self.modifiers.contains(Modifiers::ALT) {
            parts.push("Alt");
        }
        parts.push(key);
        parts.join("+")
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
/// Ctrl+Shift+letter, mnemonic per command and unclaimed by Windows.
pub fn hotkey_specs() -> [HotkeySpec; 5] {
    const CTRL_SHIFT: Modifiers = Modifiers::CONTROL.union(Modifiers::SHIFT);
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

/// The panel's window, handed over by GPUI at startup.
///
/// Not a title lookup: the overlay, the HUD and the recording frame are all
/// GPUI windows in this process, and a lookup made while a previous instance is
/// still shutting down finds that one's window instead - measured, that is how
/// the panel ended up unplaced on one launch in five.
static PANEL: OnceLock<isize> = OnceLock::new();

/// Whether the user has the panel pinned. `show_main_window` has to bounce the
/// window through `HWND_TOPMOST` to beat the foreground lock, and without this
/// it would have no way to know whether to bounce back out again.
static KEEP_ON_TOP: AtomicBool = AtomicBool::new(false);

pub fn remember_main_window(handle: isize) {
    let _ = PANEL.set(handle);
}

fn panel() -> Option<HWND> {
    PANEL.get().map(|handle| HWND(*handle as *mut _))
}

/// Centre the panel at an exact client size.
///
/// GPUI cannot be trusted with this one. It creates every window at
/// `CW_USEDEFAULT` and then applies the requested bounds - but only if the
/// centre of those bounds resolves back to the same monitor it picked, and when
/// that check fails it silently substitutes a half-screen default. Measured on
/// this machine: four launches in five opened the panel at 1300x1389 instead of
/// 400x302, and the fifth only came out right because `Window::resize` won the
/// race.
pub fn place_main_window(client_width: f32, client_height: f32) {
    let Some(window) = panel() else { return };
    let scale = unsafe { GetDpiForWindow(window) } as f32 / 96.0;
    let mut frame = RECT::default();
    let mut client = RECT::default();
    if unsafe { GetWindowRect(window, &mut frame) }.is_err()
        || unsafe { GetClientRect(window, &mut client) }.is_err()
    {
        return;
    }
    // The invisible resize border is whatever Windows says it is, so measure it
    // rather than deriving it from the style.
    let chrome_x = (frame.right - frame.left) - client.right;
    let chrome_y = (frame.bottom - frame.top) - client.bottom;
    let width = (client_width * scale).round() as i32 + chrome_x;
    let height = (client_height * scale).round() as i32 + chrome_y;

    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let work = if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        info.rcWork
    } else {
        return;
    };
    let x = work.left + ((work.right - work.left) - width) / 2;
    let y = work.top + ((work.bottom - work.top) - height) / 2;
    unsafe {
        let _ = SetWindowPos(
            window,
            None,
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
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

/// Pin the panel above other windows. A capture tool that disappears behind the
/// thing you are about to capture is a capture tool you fight.
pub fn set_keep_on_top(on: bool) {
    KEEP_ON_TOP.store(on, Ordering::Relaxed);
    if let Some(window) = panel() {
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        let after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
        unsafe {
            let _ = SetWindowPos(window, Some(after), 0, 0, 0, 0, flags);
        }
    }
}

/// Make the panel a fixed size: no resize grip, no maximize, no double-click
/// zoom.
///
/// Done by editing the HWND style rather than by `WindowOptions::is_resizable`,
/// which this GPUI build honours by dropping `WS_THICKFRAME` at creation - and
/// on a window that is already `appears_transparent` that leaves a frameless
/// style Windows never shows. Same failure as `titlebar: None`, noted in
/// `open_main_window`.
pub fn lock_window_size() {
    let Some(window) = panel() else { return };
    let style = unsafe { GetWindowLongPtrW(window, GWL_STYLE) };
    let fixed = style & !((WS_THICKFRAME.0 | WS_MAXIMIZEBOX.0) as isize);
    if fixed == style {
        return;
    }
    unsafe {
        SetWindowLongPtrW(window, GWL_STYLE, fixed);
        let _ = SetWindowPos(
            window,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Where the cursor sits inside the panel, in screen pixels from its top-left
/// corner. Recorded when a titlebar drag starts; [`drag_main_window`] puts the
/// panel back under that same point on every move.
pub fn window_drag_grab() -> Option<(i32, i32)> {
    let window = panel()?;
    let mut rect = RECT::default();
    unsafe { GetWindowRect(window, &mut rect) }.ok()?;
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&mut cursor) }.ok()?;
    Some((cursor.x - rect.left, cursor.y - rect.top))
}

/// Move the panel so the cursor keeps holding it at `grab`.
///
/// The panel is dragged by hand rather than by answering `WM_NCHITTEST` with
/// `HTCAPTION`: that hands the drag to `DefWindowProc`'s modal move loop, and
/// something inside this window cancels the loop - measured, the panel tracked
/// the cursor the whole way and then snapped back to its starting rect the
/// instant the button came up. GPUI's own `Window::start_window_move` is no
/// help either; it is implemented for Wayland and X11 only and does nothing at
/// all on Windows.
pub fn drag_main_window(grab: (i32, i32)) {
    let Some(window) = panel() else { return };
    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return;
    }
    unsafe {
        let _ = SetWindowPos(
            window,
            None,
            cursor.x - grab.0,
            cursor.y - grab.1,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

pub fn hide_main_window() {
    if let Some(window) = panel() {
        unsafe {
            let _ = ShowWindow(window, SW_HIDE);
        }
    }
}

/// Put the panel back on screen.
///
/// `hide_main_window` takes it away with `ShowWindow(SW_HIDE)`, and a hidden
/// window is not something GPUI's `activate_window` brings back - it raises and
/// focuses, both no-ops while `WS_VISIBLE` is off. So the un-hide has to happen
/// here, on the same handle the hide used.
///
/// `SetForegroundWindow` is allowed to fail: Windows refuses it when another
/// process holds the foreground lock. Bouncing through `HWND_TOPMOST` is the
/// documented way to at least get the window in front of the user, which is
/// what "Show" on a tray menu is asking for.
pub fn show_main_window() {
    let Some(window) = panel() else {
        activate_existing_window();
        return;
    };
    unsafe {
        let _ = ShowWindow(window, SW_SHOW);
        let _ = ShowWindow(window, SW_RESTORE);
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        let _ = SetWindowPos(window, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
        if !KEEP_ON_TOP.load(Ordering::Relaxed) {
            let _ = SetWindowPos(window, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);
        }
        let _ = SetForegroundWindow(window);
    }
}

/// The red rectangle that outlines what is being recorded.
///
/// One layered window with the middle cut out, not four GPUI windows. The four
/// windows were the first attempt and they measured badly: GPUI takes window
/// bounds in *logical* pixels and inflates them by the invisible resize border,
/// so a 3px physical edge came out 4px wide on the sides and 30px tall on the
/// top and bottom, offset from the region it was supposed to trace. There is no
/// GPUI knob for "give me exactly these device pixels".
///
/// `WS_EX_TRANSPARENT` keeps clicks going through to whatever is being recorded,
/// `WS_EX_NOACTIVATE` keeps it from stealing focus, and `SetWindowRgn` punches
/// out the interior so the frame never covers a pixel of the capture.
static FRAME_WINDOW: OnceLock<isize> = OnceLock::new();

fn frame_window() -> Option<HWND> {
    let existing = FRAME_WINDOW.get().map(|handle| HWND(*handle as *mut _));
    if existing.is_some() {
        return existing;
    }
    let class = w!("RapidCapRecordingFrame");
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None) }.ok()?.0);
    let brush = unsafe { CreateSolidBrush(COLORREF(frame_colour())) };
    unsafe extern "system" fn proc(
        window: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(window, message, w, l) }
    }
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(proc),
        hInstance: instance,
        lpszClassName: class,
        hbrBackground: brush,
        ..Default::default()
    };
    // A second registration of the same class fails harmlessly; the window
    // creation below is the part that has to succeed.
    unsafe { RegisterClassW(&window_class) };
    let window = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            class,
            w!("RapidCap recording frame"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
    }
    .ok()?;
    unsafe {
        let _ = SetLayeredWindowAttributes(window, COLORREF(0), 255, LWA_ALPHA);
        // Windows 11 rounds every top-level window. A traced rectangle with
        // rounded corners does not line up with the rectangle being recorded.
        let square = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &raw const square as *const _,
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
    let _ = FRAME_WINDOW.set(window.0 as isize);
    Some(window)
}

/// The frame colour as the `0x00bbggrr` GDI wants.
///
/// Accent blue, not `theme::rec()`. The frame is the biggest thing on the
/// display while a capture runs, and a red rectangle that size reads as an
/// error rather than as "this is being recorded" - ShareX draws a cool border
/// for the same reason. The red dot in the HUD still says recording.
fn frame_colour() -> u32 {
    let colour = gpui::Rgba::from(crate::theme::accent());
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    channel(colour.r) | (channel(colour.g) << 8) | (channel(colour.b) << 16)
}

/// Trace `region` with a `thickness`-pixel border, in device pixels.
pub fn show_recording_frame(region: &PhysicalRegion, thickness: u32) {
    let Some(window) = frame_window() else { return };
    let thickness = thickness.max(1) as i32;
    let width = region.width as i32 + thickness * 2;
    let height = region.height as i32 + thickness * 2;

    unsafe {
        let outer = CreateRectRgn(0, 0, width, height);
        let inner = CreateRectRgn(thickness, thickness, width - thickness, height - thickness);
        CombineRgn(Some(outer), Some(outer), Some(inner), RGN_DIFF);
        let _ = DeleteObject(inner.into());
        // Ownership of `outer` passes to the window on success.
        if SetWindowRgn(window, Some(outer), false) == 0 {
            let _ = DeleteObject(outer.into());
        }
        let _ = SetWindowPos(
            window,
            Some(HWND_TOPMOST),
            region.x - thickness,
            region.y - thickness,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

pub fn hide_recording_frame() {
    if let Some(window) = frame_window() {
        unsafe {
            let _ = ShowWindow(window, SW_HIDE);
        }
    }
}

/// Put a GPUI window exactly where it was asked to go, in device pixels.
///
/// GPUI's `window_bounds` are logical, so on any display above 100% scaling a
/// region measured in device pixels lands somewhere else entirely. Sizing the
/// `HWND` afterwards sidesteps the conversion.
pub fn place_window(handle: isize, x: i32, y: i32, width: i32, height: i32) {
    let window = HWND(handle as *mut _);
    unsafe {
        // A transparent GPUI popup still gets the Windows 11 border and corner
        // radius drawn around it - measured, that is the ghost rounded
        // rectangle that framed the recording bar.
        let square = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &raw const square as *const _,
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        let none = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_BORDER_COLOR,
            &raw const none as *const _ as *const _,
            size_of::<u32>() as u32,
        );
        // Measured on the recording bar: window 360x44, client 344x36, style
        // `0x94c00000` - `WS_CAPTION` is set even though GPUI asks for a
        // `WindowKind::PopUp` with `WINDOW_STYLE(0x0)`. That caption frame is the
        // 1px grey line across the top of the bar, and `DWMWA_BORDER_COLOR` does
        // not remove it. Stripping the frame bits does, and a popup has no
        // non-client area left to draw.
        let style = (GetWindowLongPtrW(window, GWL_STYLE) as u32 | WS_POPUP.0)
            & !(WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0);
        SetWindowLongPtrW(window, GWL_STYLE, style as isize);
        let _ = SetWindowPos(
            window,
            Some(HWND_TOPMOST),
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
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
        let value = probe_payload("C:/Captures");
        assert_eq!(value["app_id"], APP_ID);
        assert_eq!(value["output"], "C:/Captures");
        assert_eq!(value["hotkeys"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn pointer_selects_only_external_window_containing_point() {
        let rect = RECT {
            left: 100,
            top: 200,
            right: 500,
            bottom: 600,
        };
        assert!(window_candidate_contains(rect, (300, 400), 42, 7));
        assert!(!window_candidate_contains(rect, (99, 400), 42, 7));
        assert!(!window_candidate_contains(rect, (300, 400), 7, 7));
    }
}
