//! Window manipulation on Windows, via Win32.
//!
//! GPUI exposes none of this on Windows - no owner-draw always-on-top layer, no
//! drag-by-client-area, no size lock - so the panel, the overlay and the
//! recording frame are all driven by their HWNDs directly.

use std::{
    mem::size_of,
    os::windows::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    sync::OnceLock,
    thread,
    time::Duration,
};

use anyhow::Context as _;
use gpui::DisplayId;
use rapidcap_capture::{CaptureTarget, PhysicalRegion};
use windows::{
    Win32::{
        Foundation::{
            COLORREF, CloseHandle, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, HANDLE,
            HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
        },
        Graphics::{
            Dwm::{
                DWM_WINDOW_CORNER_PREFERENCE, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
                DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
                DwmGetWindowAttribute, DwmSetWindowAttribute,
            },
            Gdi::{
                CombineRgn, CreateRectRgn, CreateSolidBrush, DeleteObject, GetMonitorInfoW,
                HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
                MonitorFromWindow, RGN_DIFF, SetWindowRgn,
            },
        },
        System::LibraryLoader::GetModuleHandleW,
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_READ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
        },
        System::Threading::{
            CreateMutexW, GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        },
        UI::{
            HiDpi::GetDpiForWindow,
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, FindWindowW, GW_HWNDNEXT, GWL_STYLE,
                GetClientRect, GetCursorPos, GetSystemMetrics, GetTopWindow, GetWindow,
                GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, HWND_NOTOPMOST,
                HWND_TOPMOST, IsIconic, IsWindowVisible, LWA_ALPHA, RegisterClassW,
                SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
                SW_HIDE, SW_RESTORE, SW_SHOW, SW_SHOWNORMAL, SWP_FRAMECHANGED, SWP_NOACTIVATE,
                SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetForegroundWindow,
                SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowLongPtrW,
                SetWindowPos, ShowWindow, WDA_EXCLUDEFROMCAPTURE, WNDCLASSW, WS_CAPTION,
                WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
                WS_EX_TRANSPARENT, WS_MAXIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
            },
        },
    },
    core::{PCWSTR, PWSTR, w},
};

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
        let _ = SetWindowPos(window, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);
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
    unsafe extern "system" fn proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
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
    exclude_from_capture(window.0 as isize);
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

/// Keep a window on screen but out of anything recording that screen.
///
/// `ddagrab` reads the composited desktop, so the chrome the app holds over a
/// take - the recording bar and the frame tracing the region - was landing in
/// the file the user came for. `WDA_EXCLUDEFROMCAPTURE` cuts the window out of
/// what the duplication API hands over while leaving it visible to the person
/// using it.
///
/// Only for windows that are up *while* a recording runs. The region overlay is
/// not one: it closes before the recorder starts, has never appeared in a file,
/// and the affinity hides a window from every capture path there is - including
/// the screenshots the interaction checks take of it, which is coverage paid for
/// nothing.
///
/// Windows 10 2004 is where this affinity arrived; on anything older the call
/// fails and the chrome keeps showing up, which is the behaviour that was there
/// before. Nothing else in the recording depends on it, so a failure is not
/// worth aborting a take over.
pub fn exclude_from_capture(handle: isize) {
    // SAFETY: the window belongs to this process, which is what the call
    // requires.
    let _ = unsafe { SetWindowDisplayAffinity(HWND(handle as *mut _), WDA_EXCLUDEFROMCAPTURE) };
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

/// The monitor under the cursor, as a GPUI display and a capture rectangle.
pub fn monitor_under_cursor() -> anyhow::Result<(DisplayId, PhysicalRegion)> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }?;
    monitor_at(point)
}

/// The monitor a capture rectangle sits on, by its centre.
///
/// The cursor is the wrong question once a selection can cross monitors: the
/// pointer has moved on by the time the recording bar opens, and a region
/// dragged across a seam belongs to whichever display holds most of it - which
/// is what the centre point picks.
pub fn monitor_containing(region: &PhysicalRegion) -> anyhow::Result<(DisplayId, PhysicalRegion)> {
    monitor_at(POINT {
        x: region.x + region.width as i32 / 2,
        y: region.y + region.height as i32 / 2,
    })
}

/// The union of every monitor, in the same space `monitor_at` reports.
///
/// The origin is not (0, 0). A display placed left of or above the primary one
/// puts it negative, which is why the selection overlay is positioned from this
/// rectangle rather than sized from it.
pub fn virtual_screen() -> PhysicalRegion {
    // SAFETY: `GetSystemMetrics` takes no pointers and cannot fail - an index it
    // does not know returns 0 - and these four are documented as the bounding
    // box of the whole desktop.
    unsafe {
        PhysicalRegion {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN) as u32,
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN) as u32,
        }
    }
}

/// The monitor nearest a point, as a GPUI display and a capture rectangle.
///
/// `HMONITOR` is what GPUI's Windows backend hands out as a `DisplayId`, so the
/// handle goes straight across; `rcMonitor` is already in the virtual-screen
/// space `PhysicalRegion` uses.
fn monitor_at(point: POINT) -> anyhow::Result<(DisplayId, PhysicalRegion)> {
    let monitor: HMONITOR = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT::default(),
        rcWork: RECT::default(),
        dwFlags: 0,
    };
    unsafe { GetMonitorInfoW(monitor, &mut info) }.ok()?;
    let bounds = info.rcMonitor;
    Ok((
        DisplayId::new(monitor.0 as isize as u64),
        PhysicalRegion {
            x: bounds.left,
            y: bounds.top,
            width: (bounds.right - bounds.left) as u32,
            height: (bounds.bottom - bounds.top) as u32,
        },
    ))
}

/// Hand a folder or a file to the shell.
///
/// The verb is `open` either way: Explorer takes a directory, the registered
/// handler takes a file. Nothing here needs to know which one it was given.
pub fn open_path(path: &Path) -> anyhow::Result<()> {
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

/// Settings › Accessibility › Text size, as a multiplier.
///
/// Windows applies this to its own text and to nothing an app draws itself, so
/// a custom titlebar stays the size it was authored while every native one on
/// the machine grows. The titlebar reads this and scales with them.
///
/// Stored as a percentage from 100 to 225. Absent means nobody has moved the
/// slider, and the clamp keeps a hand-edited registry value from producing a
/// titlebar taller than the panel.
pub fn text_scale() -> f32 {
    static SCALE: OnceLock<f32> = OnceLock::new();
    *SCALE.get_or_init(|| read_text_scale_percent().map_or(1.0, scale_from_percent))
}

fn scale_from_percent(percent: u32) -> f32 {
    (percent as f32 / 100.0).clamp(1.0, 2.25)
}

fn read_text_scale_percent() -> Option<u32> {
    let mut key = HKEY::default();
    // SAFETY: `w!` strings are NUL-terminated and static, and the key is closed
    // on every path out.
    if unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Accessibility"),
            None,
            KEY_READ,
            &mut key,
        )
    } != ERROR_SUCCESS
    {
        return None;
    }
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `value` is a `u32` and `size` says so, which is what the DWORD
    // read expects.
    let queried = unsafe {
        RegQueryValueExW(
            key,
            w!("TextScaleFactor"),
            None,
            None,
            Some(&raw mut value as *mut u8),
            Some(&mut size),
        )
    };
    // SAFETY: `key` was opened above and is not used again.
    let _ = unsafe { RegCloseKey(key) };
    (queried == ERROR_SUCCESS).then_some(value)
}

#[cfg(test)]
mod text_scale_tests {
    use super::scale_from_percent;

    #[test]
    fn the_sizes_settings_offers_arrive_unchanged() {
        for (percent, expected) in [(100, 1.0), (125, 1.25), (150, 1.5), (225, 2.25)] {
            assert_eq!(scale_from_percent(percent), expected);
        }
    }

    #[test]
    fn a_value_outside_the_slider_is_clamped_rather_than_trusted() {
        // A hand-edited registry can say anything. Below 100 the titlebar would
        // shrink under its own glyphs; above 225 it would be taller than the
        // panel it sits on.
        assert_eq!(scale_from_percent(0), 1.0);
        assert_eq!(scale_from_percent(50), 1.0);
        assert_eq!(scale_from_percent(10_000), 2.25);
    }
}
